//! Installing a Claudepot verb as a Claude Code hook, and taking it out.
//!
//! Two features hook Claude Code: remote approvals (`remote::approval`,
//! event `PermissionRequest`) and permission grants (`permission::hook`,
//! event `PreToolUse`). Each is one entry in the user's
//! `~/.claude/settings.json` pointing back at this binary. The shape of
//! that entry, and the care around writing it, is the same for both, so
//! it lives here once. A second copy of "find our entry among the
//! user's hooks and replace it" is a second place for the removal to
//! leave litter or eat a stranger's hook.
//!
//! Every write goes through [`crate::settings_mutex`], which is the
//! only sanctioned way to read-modify-write a CC settings file. A
//! second writer doing its own RMW would silently discard whichever
//! mutation landed first.
//!
//! **The exec form (`command` + `args`) is deliberate.** CC resolves
//! `command` as an executable and spawns it directly with these
//! arguments, with no shell anywhere. A binary path containing a space,
//! a quote, a `$` or a backtick therefore never reaches a shell parser
//! — the failure this repo has already paid for once, in a commit
//! message that executed its own backticks.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value as JsonValue};

use crate::paths::claude_config_dir;
use crate::settings_mutex::{self, Change, Mutation, SettingsMutexError};

/// One Claudepot hook: which CC event it answers, the verb that
/// identifies it among the user's own hooks, and how long CC may wait
/// on it before killing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookSpec {
    /// The CC hook event, e.g. `PermissionRequest`.
    pub event: &'static str,
    /// The `claudepot` subcommand, e.g. `["hook", "permission-request"]`.
    /// Matched to recognise our entry regardless of where the binary
    /// lives — see [`is_ours`].
    pub verb: &'static [&'static str],
    /// Written as the entry's `timeout`. CC kills the hook at it, and a
    /// killed `PreToolUse` hook blocks the tool call, so every verb
    /// must finish well inside its own number.
    pub timeout_secs: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("settings: {0}")]
    Settings(#[from] SettingsMutexError),
    #[error("serialize hook entry: {0}")]
    Json(#[from] serde_json::Error),
    /// `hooks` (or the event under it) exists and is not the shape CC
    /// defines. Refused rather than overwritten: it is the user's file,
    /// and silently replacing a key we cannot read is how an editor
    /// eats a config.
    #[error("{}: `{key}` is not the expected shape — fix it by hand, or remove it", crate::paths::claude_config_dir().join("settings.json").display())]
    Malformed { key: &'static str },
}

fn user_settings_path() -> PathBuf {
    claude_config_dir().join("settings.json")
}

/// The CLI binary's `[[bin]] name` — the one executable that carries
/// the `hook …` verbs. The GUI is `claudepot-tauri` (or `Claudepot`
/// inside a bundle) and has no clap parser at all.
const CLI_STEM: &str = "claudepot";

#[derive(Debug, thiserror::Error)]
pub enum HookBinaryError {
    #[error("cannot locate this binary: {0}")]
    CurrentExe(std::io::Error),
    #[error("{0}")]
    Sibling(#[from] crate::mcp_probe::McpProbeError),
}

/// Is `path` the CLI itself, judged by its file stem?
fn is_cli_binary(path: &Path) -> bool {
    path.file_stem().and_then(|s| s.to_str()) == Some(CLI_STEM)
}

/// The executable a hook entry must point at: the `claudepot` CLI.
///
/// `std::env::current_exe()` is right only when the caller **is** the
/// CLI (`claudepot remote serve` from a terminal). From the GUI it is
/// the Tauri app, which has no `hook` verb — and CC does not report a
/// hook that prints nothing as an error, so an entry aimed at the GUI
/// would have CC launch the desktop app on every tool call, wait out
/// the timeout, and for a `PreToolUse` hook block the call. Both
/// Claudepot hooks were installed that way from the GUI before this
/// resolver existed; the remote approval one shipped like it.
///
/// Order: `CLAUDEPOT_CLI_PATH` (explicit override, same as the agent
/// shim honours), then `current_exe` if it is the CLI, then the CLI
/// bundled beside the GUI (`mcp_probe::cli_candidates` — the sidecar
/// name differs between a dev tree and a release bundle). There is no
/// last-resort fallback to `current_exe`: an entry pointing at the
/// wrong binary is worse than no entry, so this fails instead.
pub fn hook_binary() -> Result<PathBuf, HookBinaryError> {
    if let Some(p) = std::env::var_os("CLAUDEPOT_CLI_PATH") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Ok(p);
        }
    }
    let current = std::env::current_exe().map_err(HookBinaryError::CurrentExe)?;
    resolve_hook_binary(&current, cfg!(debug_assertions))
}

/// The judgement behind [`hook_binary`], with the running executable
/// injected so it can be tested against paths that are not this test
/// binary.
fn resolve_hook_binary(current: &Path, debug: bool) -> Result<PathBuf, HookBinaryError> {
    if is_cli_binary(current) {
        return Ok(current.to_path_buf());
    }
    let dir = current.parent().unwrap_or(current);
    Ok(crate::mcp_probe::resolve_sibling_cli(dir, debug)?)
}

/// Our hook entry for `spec`, pointing at `binary`.
pub fn entry(spec: &HookSpec, binary: &Path) -> JsonValue {
    json!({
        "type": "command",
        "command": binary.to_string_lossy(),
        "args": spec.verb,
        "timeout": spec.timeout_secs,
    })
}

/// Is this one of ours?
///
/// Matched on the verb rather than the binary path, so an entry written
/// by a Claudepot that has since moved or been reinstalled is still
/// recognised as ours to replace or remove — rather than left behind as
/// a stranger's hook we politely decline to touch.
pub fn is_ours(spec: &HookSpec, hook: &JsonValue) -> bool {
    hook.get("args")
        .and_then(|a| a.as_array())
        .is_some_and(|a| {
            a.len() >= spec.verb.len()
                && a.iter()
                    .zip(spec.verb.iter())
                    .all(|(got, want)| got.as_str() == Some(*want))
        })
}

/// Strip our entries for `spec` from a `hooks` object, returning
/// whether anything went. Leaves the user's own entries, and prunes
/// containers we emptied so an uninstall does not leave
/// `"PermissionRequest": []` behind as litter.
pub fn remove_ours(spec: &HookSpec, root: &mut Map<String, JsonValue>) -> bool {
    let Some(hooks) = root.get_mut("hooks").and_then(|h| h.as_object_mut()) else {
        return false;
    };
    let Some(groups) = hooks.get_mut(spec.event).and_then(|g| g.as_array_mut()) else {
        return false;
    };

    let mut changed = false;
    for group in groups.iter_mut() {
        if let Some(list) = group.get_mut("hooks").and_then(|l| l.as_array_mut()) {
            let before = list.len();
            list.retain(|h| !is_ours(spec, h));
            changed |= list.len() != before;
        }
    }
    // A group whose only hook was ours is now an empty group.
    let before = groups.len();
    groups.retain(|g| {
        g.get("hooks")
            .and_then(|l| l.as_array())
            .is_none_or(|l| !l.is_empty())
    });
    changed |= groups.len() != before;

    if groups.is_empty() {
        hooks.remove(spec.event);
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    changed
}

/// Every entry of ours for `spec` under the event, in file order.
fn owned<'a>(spec: &HookSpec, root: &'a JsonValue) -> Vec<&'a JsonValue> {
    root.get("hooks")
        .and_then(|h| h.get(spec.event))
        .and_then(|g| g.as_array())
        .map(|groups| {
            groups
                .iter()
                .filter_map(|g| g.get("hooks").and_then(|l| l.as_array()))
                .flatten()
                .filter(|h| is_ours(spec, h))
                .collect()
        })
        .unwrap_or_default()
}

/// Healthy means **exactly one** entry of ours, and it is `want`. Two
/// owned entries — one current, one left by an older Claudepot or a
/// hand-edit — are not healthy: CC runs every entry under the event,
/// so the stale one still fires, points at a binary that may be gone,
/// and for a `PreToolUse` hook a missing binary is a failed hook.
fn is_healthy(ours: &[&JsonValue], want: &JsonValue) -> bool {
    matches!(ours, [only] if *only == want)
}

/// Is an entry of ours for `spec` present, and does it point at
/// `binary`? `Some(true)` = installed and current (exactly one owned
/// entry, equal to what `install` would write), `Some(false)` =
/// present but wrong (pointing elsewhere, or duplicated), `None` =
/// absent or unreadable.
pub fn installed_state(spec: &HookSpec, binary: &Path) -> Option<bool> {
    let want = entry(spec, binary);
    let bytes = std::fs::read(user_settings_path()).ok()?;
    let root: JsonValue = serde_json::from_slice(&bytes).ok()?;
    let ours = owned(spec, &root);
    if ours.is_empty() {
        return None;
    }
    Some(is_healthy(&ours, &want))
}

/// Add the hook, replacing any earlier copy of ours.
///
/// Idempotent: installing twice leaves exactly one entry, and
/// installing after the binary moved rewrites the path rather than
/// adding a second entry pointing at a file that is gone.
pub fn install(spec: &HookSpec, binary: &Path) -> Result<Mutation<()>, InstallError> {
    let want = entry(spec, binary);
    settings_mutex::mutate_settings_file(&user_settings_path(), |root, _was| {
        // Already exactly right — one entry of ours, and it is `want`?
        // Then write nothing: an unchanged file keeps its mtime, and a
        // reconcile that runs every tick must not look like someone
        // edited the user's settings. Anything else, including a
        // current entry with a stale duplicate beside it, is rewritten
        // to exactly one.
        let snapshot = JsonValue::Object(root.clone());
        if is_healthy(&owned(spec, &snapshot), &want) {
            return Ok::<_, InstallError>(Change::Skip(()));
        }

        remove_ours(spec, root);
        let hooks = root
            .entry("hooks")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or(InstallError::Malformed { key: "hooks" })?;
        let groups = hooks
            .entry(spec.event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or(InstallError::Malformed { key: spec.event })?;
        // No `matcher`: every call of the event, which is the whole
        // point — a prompt this hook skipped would wait at the keyboard
        // with nothing to say why.
        groups.push(json!({ "hooks": [want.clone()] }));
        Ok(Change::Write(()))
    })
}

/// Take the hook back out. Safe to call when it was never installed.
pub fn uninstall(spec: &HookSpec) -> Result<Mutation<()>, InstallError> {
    settings_mutex::mutate_settings_file(&user_settings_path(), |root, _was| {
        if remove_ours(spec, root) {
            Ok::<_, InstallError>(Change::Write(()))
        } else {
            Ok(Change::Skip(()))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: HookSpec = HookSpec {
        event: "PermissionRequest",
        verb: &["hook", "permission-request"],
        timeout_secs: 120,
    };

    const OTHER: HookSpec = HookSpec {
        event: "PreToolUse",
        verb: &["hook", "pre-tool-use"],
        timeout_secs: 10,
    };

    fn ours() -> JsonValue {
        entry(&SPEC, Path::new("/usr/local/bin/claudepot"))
    }

    fn theirs() -> JsonValue {
        json!({"type": "command", "command": "/bin/echo", "args": ["hi"]})
    }

    #[test]
    fn the_entry_uses_the_exec_form_so_no_shell_ever_sees_the_path() {
        let e = entry(&SPEC, Path::new("/Users/a b/`whoami`/claudepot"));
        assert_eq!(e["command"], json!("/Users/a b/`whoami`/claudepot"));
        assert_eq!(e["args"], json!(["hook", "permission-request"]));
        assert_eq!(e["type"], json!("command"));
        assert_eq!(e["timeout"], json!(120));
    }

    #[test]
    fn ours_is_recognised_by_verb_not_by_path() {
        assert!(is_ours(
            &SPEC,
            &entry(&SPEC, Path::new("/anywhere/else/claudepot"))
        ));
        assert!(!is_ours(&SPEC, &theirs()));
        // A near-miss must not be adopted.
        assert!(!is_ours(
            &SPEC,
            &json!({"args": ["hook", "something-else"]})
        ));
        assert!(!is_ours(&SPEC, &json!({"args": ["hook"]})));
        assert!(!is_ours(
            &SPEC,
            &json!({"command": "claudepot hook permission-request"})
        ));
        // And one feature's entry is not the other's.
        assert!(!is_ours(&SPEC, &entry(&OTHER, Path::new("/x"))));
        assert!(!is_ours(&OTHER, &ours()));
    }

    #[test]
    fn removing_ours_leaves_the_users_own_hooks_alone() {
        let mut root: Map<String, JsonValue> = serde_json::from_value(json!({
            "hooks": {"PermissionRequest": [{"hooks": [theirs(), ours()]}]}
        }))
        .unwrap();

        assert!(remove_ours(&SPEC, &mut root));
        assert_eq!(
            root["hooks"]["PermissionRequest"][0]["hooks"],
            json!([theirs()])
        );
    }

    #[test]
    fn an_uninstall_leaves_no_litter_behind() {
        // The whole `hooks` key was ours; it should be gone, not an
        // empty husk that reads as configuration.
        let mut root: Map<String, JsonValue> = serde_json::from_value(json!({
            "model": "opus",
            "hooks": {"PermissionRequest": [{"hooks": [ours()]}]}
        }))
        .unwrap();

        assert!(remove_ours(&SPEC, &mut root));
        assert!(!root.contains_key("hooks"), "{root:?}");
        assert_eq!(root["model"], json!("opus"), "and nothing else was touched");
    }

    #[test]
    fn other_events_survive_our_uninstall() {
        let mut root: Map<String, JsonValue> = serde_json::from_value(json!({
            "hooks": {
                "PermissionRequest": [{"hooks": [ours()]}],
                "PreToolUse": [{"hooks": [theirs()]}]
            }
        }))
        .unwrap();

        assert!(remove_ours(&SPEC, &mut root));
        assert!(root["hooks"].get("PermissionRequest").is_none());
        assert!(root["hooks"].get("PreToolUse").is_some());
    }

    #[test]
    fn one_features_uninstall_spares_the_others_entry() {
        // Both Claudepot hooks live in the same file. Taking the
        // approval hook out when `remote serve` stops must not take a
        // live permission grant's hook with it.
        let mut root: Map<String, JsonValue> = serde_json::from_value(json!({
            "hooks": {
                "PermissionRequest": [{"hooks": [ours()]}],
                "PreToolUse": [{"hooks": [entry(&OTHER, Path::new("/x"))]}]
            }
        }))
        .unwrap();
        assert!(remove_ours(&SPEC, &mut root));
        assert!(root["hooks"].get("PermissionRequest").is_none());
        assert!(is_ours(&OTHER, &root["hooks"]["PreToolUse"][0]["hooks"][0]));
    }

    #[test]
    fn removing_from_a_file_that_never_had_us_changes_nothing() {
        let mut empty: Map<String, JsonValue> = Map::new();
        assert!(!remove_ours(&SPEC, &mut empty));

        let mut theirs_only: Map<String, JsonValue> = serde_json::from_value(json!({
            "hooks": {"PermissionRequest": [{"hooks": [theirs()]}]}
        }))
        .unwrap();
        assert!(!remove_ours(&SPEC, &mut theirs_only));
        assert_eq!(
            theirs_only["hooks"]["PermissionRequest"][0]["hooks"],
            json!([theirs()])
        );
    }

    mod binary {
        use super::*;

        #[test]
        fn the_cli_itself_is_used_as_is() {
            let d = tempfile::tempdir().unwrap();
            let cli = d.path().join("claudepot");
            std::fs::write(&cli, b"").unwrap();
            assert_eq!(resolve_hook_binary(&cli, true).unwrap(), cli);
            let exe = d.path().join("claudepot.exe");
            std::fs::write(&exe, b"").unwrap();
            assert_eq!(resolve_hook_binary(&exe, false).unwrap(), exe);
        }

        #[test]
        fn the_gui_resolves_to_the_cli_beside_it_and_never_to_itself() {
            // The bug this exists for: from the GUI, `current_exe` is
            // the Tauri app. In a dev tree the CLI sits beside it as
            // `claudepot`; in a bundle as `claudepot-cli`.
            let d = tempfile::tempdir().unwrap();
            let gui = d.path().join("claudepot-tauri");
            std::fs::write(&gui, b"").unwrap();
            assert!(
                resolve_hook_binary(&gui, true).is_err(),
                "no CLI beside the GUI must be an error, not the GUI"
            );
            let cli = d.path().join("claudepot");
            std::fs::write(&cli, b"").unwrap();
            assert_eq!(resolve_hook_binary(&gui, true).unwrap(), cli);

            let bundle = tempfile::tempdir().unwrap();
            let app = bundle.path().join("Claudepot");
            std::fs::write(&app, b"").unwrap();
            let sidecar = bundle.path().join("claudepot-cli");
            std::fs::write(&sidecar, b"").unwrap();
            assert_eq!(resolve_hook_binary(&app, false).unwrap(), sidecar);
        }

        #[test]
        fn the_stem_test_is_exact() {
            assert!(is_cli_binary(Path::new("/x/claudepot")));
            // `Path::file_stem` is host-specific: a backslash is a
            // separator only on Windows, so the drive-letter form is a
            // Windows-only assertion (rules/paths.md — OS-specific
            // behaviour is cfg-gated, pure string ops are not).
            #[cfg(windows)]
            assert!(is_cli_binary(Path::new(r"C:\x\claudepot.exe")));
            assert!(!is_cli_binary(Path::new("/x/claudepot-tauri")));
            assert!(!is_cli_binary(Path::new("/x/claudepot-cli")));
            assert!(!is_cli_binary(Path::new(
                "/Applications/Claudepot.app/Contents/MacOS/Claudepot"
            )));
        }
    }

    /// The file-level behaviour, on a real settings file under an
    /// isolated `CLAUDE_CONFIG_DIR`.
    mod on_disk {
        use super::*;

        fn isolated() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
            let lock = crate::testing::lock_data_dir();
            let tmp = tempfile::tempdir().unwrap();
            std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path());
            (tmp, lock)
        }

        #[test]
        fn install_is_idempotent_and_rewrites_a_moved_binary_path() {
            // A grant can outlive a Homebrew upgrade or an app move. The
            // reconcile that runs every tick must repair the path, not
            // stack a second entry beside one pointing at nothing.
            let (_t, _l) = isolated();
            install(&SPEC, Path::new("/old/claudepot")).unwrap();
            install(&SPEC, Path::new("/old/claudepot")).unwrap();
            assert_eq!(
                installed_state(&SPEC, Path::new("/old/claudepot")),
                Some(true)
            );

            install(&SPEC, Path::new("/new/claudepot")).unwrap();
            assert_eq!(
                installed_state(&SPEC, Path::new("/new/claudepot")),
                Some(true)
            );
            assert_eq!(
                installed_state(&SPEC, Path::new("/old/claudepot")),
                Some(false),
                "the old path is gone, not kept beside the new one"
            );
            let root: JsonValue =
                serde_json::from_slice(&std::fs::read(user_settings_path()).unwrap()).unwrap();
            let entries: Vec<&JsonValue> = root["hooks"]["PermissionRequest"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|g| g["hooks"].as_array().unwrap())
                .collect();
            assert_eq!(entries.len(), 1);
        }

        #[test]
        fn a_stale_duplicate_beside_a_current_entry_is_collapsed_to_one() {
            // An older Claudepot, or a hand-edit, can leave two entries
            // of ours under one event. CC runs both; the stale one
            // points at a binary that may be gone. `install` used to
            // see the current one and skip, leaving the stale one to
            // fire on every call.
            let (_t, _l) = isolated();
            let mut root: Map<String, JsonValue> = Map::new();
            root.insert(
                "hooks".into(),
                json!({ SPEC.event: [
                    {"hooks": [entry(&SPEC, Path::new("/stale/claudepot"))]},
                    {"hooks": [entry(&SPEC, Path::new("/current/claudepot"))]},
                ]}),
            );
            std::fs::write(
                user_settings_path(),
                serde_json::to_vec(&JsonValue::Object(root)).unwrap(),
            )
            .unwrap();
            assert_eq!(
                installed_state(&SPEC, Path::new("/current/claudepot")),
                Some(false),
                "two owned entries are not a healthy install"
            );

            install(&SPEC, Path::new("/current/claudepot")).unwrap();
            assert_eq!(
                installed_state(&SPEC, Path::new("/current/claudepot")),
                Some(true)
            );
            let root: JsonValue =
                serde_json::from_slice(&std::fs::read(user_settings_path()).unwrap()).unwrap();
            assert_eq!(owned(&SPEC, &root).len(), 1);
        }

        #[test]
        fn uninstall_removes_only_this_specs_entry() {
            let (_t, _l) = isolated();
            install(&SPEC, Path::new("/x")).unwrap();
            install(&OTHER, Path::new("/x")).unwrap();
            uninstall(&SPEC).unwrap();
            assert_eq!(installed_state(&SPEC, Path::new("/x")), None);
            assert_eq!(installed_state(&OTHER, Path::new("/x")), Some(true));
            uninstall(&OTHER).unwrap();
            assert_eq!(installed_state(&OTHER, Path::new("/x")), None);
            let root: JsonValue =
                serde_json::from_slice(&std::fs::read(user_settings_path()).unwrap()).unwrap();
            assert!(root.get("hooks").is_none(), "{root}");
        }

        #[test]
        fn installed_state_is_none_with_no_settings_file() {
            let (_t, _l) = isolated();
            assert_eq!(installed_state(&SPEC, Path::new("/x")), None);
        }
    }
}
