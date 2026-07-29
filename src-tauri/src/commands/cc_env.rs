//! IPC commands for Global → Config → Env Variables.
//!
//! Thin wrappers over `claudepot_core::cc_env`, per `rules/architecture.md`:
//! resolution, validation, and the value projection all live in core, and this
//! layer only resolves the installed Claude Code binary, moves values across
//! the bridge, and maps errors to strings.
//!
//! # Secret direction
//!
//! Tauri 2 IPC is in-process, so the bridge is not a cross-trust boundary —
//! but *direction* still decides what is acceptable. Secrets arriving from a
//! paste are fine and are zeroized on every exit path, exactly as
//! `key_*_add` and `settings_github_token_set` do. Secrets *returning* are
//! not: they would sit in the JS heap waiting for a DevTools snapshot. So
//! [`cc_env_list`] emits `SecretSet` or `Absent` for any secret-capable
//! variable and never its bytes, and the value is not echoed back by
//! [`cc_env_set`] either.
//!
//! Note that the `EnvOverview` returned by every command here is the same
//! shape `cc_env_list` returns — every write hands back the authoritative
//! post-write state so the renderer reconciles against the file rather than
//! against its own optimism.

use claudepot_core::cc_env::{self, EnvOverview};
use zeroize::Zeroizing;

fn load() -> Result<EnvOverview, String> {
    // Binary selection is core's policy (`resolve_installed_claude`); this
    // layer only calls it, per rules/architecture.md.
    let (version, path) = cc_env::resolve_installed_claude();
    cc_env::load(version.as_deref(), path.as_deref()).map_err(|e| e.to_string())
}

/// `cc_env_list` — the spec, every variable's resolved state, and the three
/// buckets, in one trip.
///
/// **Serializes no secret bytes, ever.** A secret-capable variable emits
/// `SecretSet` or `Absent`; an unrecognized key emits `Withheld` with its
/// JSON shape and nothing else.
#[tauri::command]
pub async fn cc_env_list() -> Result<EnvOverview, String> {
    tokio::task::spawn_blocking(load)
        .await
        .map_err(|e| format!("cc_env_list join: {e}"))?
}

/// `cc_env_set` — write one documented, editable variable.
///
/// Core rejects an unknown name, a blocked name, an out-of-enum value, and a
/// non-integer for a number. Numbers validate syntax only: the spec carries
/// no bounds, and inventing them would reject values Claude Code accepts.
///
/// `value` may be a credential, so every owned copy is zeroized on every exit
/// path — success, error, and a panicking or cancelled blocking task alike.
///
/// `Zeroizing` rather than a hand-placed `.zeroize()`: the earlier shape put
/// the scrub after the `?` on the join result, so a panicked task took the
/// early return and left the plaintext in the heap. A guard that runs in
/// `Drop` cannot be skipped by a return path someone adds later.
#[tauri::command]
pub async fn cc_env_set(name: String, value: String) -> Result<EnvOverview, String> {
    let value = Zeroizing::new(value);
    let result = {
        let name = name.clone();
        let owned = Zeroizing::new(value.to_string());
        tokio::task::spawn_blocking(move || {
            cc_env::set_user_env_var(&name, &owned).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| format!("cc_env_set join: {e}"))?
    };
    result?;
    tokio::task::spawn_blocking(load)
        .await
        .map_err(|e| format!("cc_env_set join: {e}"))?
}

/// `cc_env_clear` — remove one key from the `env` map.
///
/// Accepts **any name actually present**, documented or not, after exact-name
/// validation — that is what makes a hand-set key removable, and it resolves
/// the contradiction between "the pane can clear unknown keys" and "core
/// rejects unknown names" (the rejection belongs to `set`, not to `clear`).
/// Blocked names are still refused: their rows are read-only, and a pane that
/// cannot set a value should not be able to unset one either.
///
/// This never takes effect in a running session — Claude Code re-applies
/// `settings.env` additively and deletes nothing — so the confirmation that
/// precedes this call has to say the old value survives until relaunch.
#[tauri::command]
pub async fn cc_env_clear(name: String) -> Result<EnvOverview, String> {
    tokio::task::spawn_blocking(move || {
        cc_env::clear_user_env_var(&name).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("cc_env_clear join: {e}"))??;
    tokio::task::spawn_blocking(load)
        .await
        .map_err(|e| format!("cc_env_clear join: {e}"))?
}

#[cfg(test)]
mod tests {
    use claudepot_core::cc_env::{
        self, clear_user_env_var, resolve_all, set_user_env_var, spec, EnvValue,
    };
    use serde_json::{json, Map, Value};

    /// Point `CLAUDE_CONFIG_DIR` at a temp dir for the duration of a test.
    /// The variable is process-global, so the guard is a real static —
    /// `Mutex::new(())` at the call site would exclude nothing.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct Isolated {
        _dir: tempfile::TempDir,
        _guard: std::sync::MutexGuard<'static, ()>,
        prev: Option<std::ffi::OsString>,
    }

    impl Isolated {
        fn new() -> Self {
            let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = tempfile::tempdir().unwrap();
            let prev = std::env::var_os("CLAUDE_CONFIG_DIR");
            std::env::set_var("CLAUDE_CONFIG_DIR", dir.path());
            Self {
                _dir: dir,
                _guard: guard,
                prev,
            }
        }
    }

    impl Drop for Isolated {
        fn drop(&mut self) {
            match self.prev.take() {
                Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
                None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
            }
        }
    }

    /// The write-then-reload contract these commands wrap, exercised through
    /// the same core calls they make. The async wrappers add only
    /// `spawn_blocking` and error mapping, which a Tauri runtime would be
    /// needed to drive; what is worth locking down is that a write lands and
    /// the reload reports it *redacted*.
    #[test]
    fn a_written_secret_comes_back_as_set_but_withheld() {
        let _iso = Isolated::new();

        set_user_env_var("ANTHROPIC_API_KEY", "sk-ant-oat01-written").unwrap();
        let overview = cc_env::load(None, None).unwrap();

        let row = overview
            .documented
            .iter()
            .find(|v| v.spec.name == "ANTHROPIC_API_KEY")
            .unwrap();
        assert_eq!(row.settings_value, EnvValue::SecretSet);

        let payload = serde_json::to_string(&overview).unwrap();
        assert!(!payload.contains("sk-ant"), "the reload echoed the secret");

        // …and the value really is on disk, so "withheld" is not "not written".
        let raw = std::fs::read_to_string(cc_env::user_settings_path()).unwrap();
        assert!(raw.contains("sk-ant-oat01-written"));
    }

    #[test]
    fn a_rejected_write_leaves_no_file_and_never_quotes_the_value() {
        let _iso = Isolated::new();

        let err = set_user_env_var("MAX_THINKING_TOKENS", "sk-ant-oat01-pasted").unwrap_err();
        let msg = err.to_string();
        assert!(!msg.contains("sk-ant"), "{msg}");
        assert!(!cc_env::user_settings_path().exists());

        assert!(set_user_env_var("NOT_A_REAL_VARIABLE", "1").is_err());
        assert!(set_user_env_var("CLAUDE_CONFIG_DIR", "/tmp/x").is_err());
    }

    #[test]
    fn clear_round_trips_and_refuses_the_cases_it_should() {
        let _iso = Isolated::new();

        set_user_env_var("USE_BUILTIN_RIPGREP", "0").unwrap();
        assert!(clear_user_env_var("USE_BUILTIN_RIPGREP").unwrap().wrote);
        // Gone, and clearing again is an error rather than a silent success.
        assert!(clear_user_env_var("USE_BUILTIN_RIPGREP").is_err());
        // A blocked name stays refused even though it is a real variable.
        assert!(clear_user_env_var("CLAUDE_CONFIG_DIR").is_err());
    }

    /// The IPC contract, asserted on the exact payload the bridge serializes:
    /// a settings file setting every secret-capable variable (both
    /// `SAFE_ENV_VARS` overlaps included), a nested object value, and a
    /// `~/.claude.json` secret. Zero secret bytes may appear in the JSON.
    ///
    /// This is the gate that has to be green before any renderer work — the
    /// core-level sibling lives in `cc_env::state`, and this one covers the
    /// serialization that actually crosses the bridge.
    #[test]
    fn cc_env_list_payload_contains_no_secret_bytes() {
        const MARKER: &str = "SECRET-CANARY-ipc-4c71";

        let mut env: Map<String, Value> = Map::new();
        let mut secret_names = Vec::new();
        for v in spec::spec().vars.iter().filter(|v| v.safety.secret) {
            secret_names.push(v.name.clone());
            env.insert(v.name.clone(), json!(format!("sk-ant-{MARKER}")));
        }
        assert!(secret_names.iter().any(|n| n == "ANTHROPIC_CUSTOM_HEADERS"));
        assert!(secret_names
            .iter()
            .any(|n| n == "ANTHROPIC_FOUNDRY_API_KEY"));

        env.insert(
            "ANTHROPIC_BASE_URL".into(),
            json!({ "Authorization": format!("Bearer {MARKER}") }),
        );
        env.insert("A_FUTURE_CREDENTIAL".into(), json!(MARKER));

        let mut legacy: Map<String, Value> = Map::new();
        legacy.insert("ANTHROPIC_AUTH_TOKEN".into(), json!(format!("l-{MARKER}")));

        let overview = resolve_all(&env, &legacy, None, None);
        let payload = serde_json::to_string(&overview).expect("serializes");

        assert!(!payload.contains(MARKER), "a secret crossed the bridge");
        assert!(!payload.contains("sk-ant"), "a secret crossed the bridge");

        // Withholding must not read as "not set".
        for name in &secret_names {
            let row = overview
                .documented
                .iter()
                .find(|v| &v.spec.name == name)
                .unwrap();
            assert_eq!(row.settings_value, EnvValue::SecretSet, "{name}");
        }
    }
}
