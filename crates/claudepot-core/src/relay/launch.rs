//! The environment `claudepot relay run` launches Claude Code with
//! (plan §8). It is authoritative: whatever the caller's shell carried
//! about providers and credentials is removed, and only the relay's
//! values are set.
//!
//! - Set: `CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST=1` (so settings-file env
//!   cannot redirect the base URL — verified in P0), `ANTHROPIC_BASE_URL`
//!   = the server plus `/u/<upstream>`, `ANTHROPIC_AUTH_TOKEN` = the
//!   device token, `NODE_EXTRA_CA_CERTS` = a 0600 file holding the pinned
//!   CA, and `CLAUDE_CONFIG_DIR` = an isolated profile per upstream.
//! - Removed: every variable `cc-env-spec.json` marks provider-managed
//!   (provider selectors, model pins, credentials), every secret in its
//!   `auth` category, and `ANTHROPIC_CUSTOM_HEADERS`, which may carry an
//!   `Authorization` header for some other service and would then be sent
//!   to the relay. The list comes from the spec, not from a hand-typed one.
//!
//! Disclosed exposure: the device token is in CC's environment, so CC's
//! tool subprocesses can read it. It buys inference on this relay and
//! nothing else, and the server can revoke it.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::cc_env::spec::EnvSpec;

use super::secret::Secret;
use super::target::UpstreamId;

/// Per-upstream CC profiles, under the data dir.
pub const PROFILES_DIR: &str = "relay-profiles";

/// Prefixed, because an upstream id is a server's choice and `con`,
/// `nul` or `aux` would otherwise name a reserved Windows device: the
/// id alphabet (`[a-z0-9-]`, no leading `-`) can never make `u-<id>`
/// reserved, and keeps it a single path component on every OS.
pub fn profile_dir(data_dir: &Path, upstream: &UpstreamId) -> PathBuf {
    data_dir
        .join(PROFILES_DIR)
        .join(format!("u-{}", upstream.as_str()))
}

/// Removed even though the spec does not mark them provider-managed.
const ALSO_CLEARED: &[&str] = &["ANTHROPIC_CUSTOM_HEADERS"];

/// The variables a relay launch removes from the inherited environment.
pub fn cleared_vars(spec: &EnvSpec) -> Vec<String> {
    let mut out: Vec<String> = spec
        .vars
        .iter()
        .filter(|v| v.safety.provider_managed || (v.safety.secret && v.category == "auth"))
        .map(|v| v.name.clone())
        .chain(ALSO_CLEARED.iter().map(|s| (*s).to_owned()))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// `base` with the upstream's path prefix. Always explicit, so a server
/// changing its default cannot move a running session to another
/// upstream.
pub fn base_url_for(base: &reqwest::Url, upstream: &UpstreamId) -> String {
    format!(
        "{}/u/{}",
        base.as_str().trim_end_matches('/'),
        upstream.as_str()
    )
}

pub struct LaunchEnv {
    pub remove: Vec<String>,
    /// Non-secret values.
    pub set: Vec<(String, OsString)>,
    pub token: Secret,
}

impl std::fmt::Debug for LaunchEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchEnv")
            .field("remove", &self.remove)
            .field("set", &self.set)
            .field("token", &self.token)
            .finish()
    }
}

pub fn build(
    spec: &EnvSpec,
    base_url: &str,
    token: Secret,
    ca_bundle: Option<&Path>,
    config_dir: &Path,
) -> LaunchEnv {
    let mut set: Vec<(String, OsString)> = vec![
        ("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST".into(), "1".into()),
        ("ANTHROPIC_BASE_URL".into(), base_url.into()),
        ("CLAUDE_CONFIG_DIR".into(), config_dir.into()),
    ];
    if let Some(p) = ca_bundle {
        set.push(("NODE_EXTRA_CA_CERTS".into(), p.into()));
    }
    LaunchEnv {
        remove: cleared_vars(spec),
        set,
        token,
    }
}

impl LaunchEnv {
    /// Apply to a command: remove first, then set, so a cleared variable
    /// the relay also sets ends with the relay's value.
    pub fn apply(&self, cmd: &mut tokio::process::Command) {
        for k in &self.remove {
            cmd.env_remove(k);
        }
        for (k, v) in &self.set {
            cmd.env(k, v);
        }
        cmd.env("ANTHROPIC_AUTH_TOKEN", self.token.expose());
    }
}

/// The pinned CA as a 0600 file. Deleted when the handle drops, so the
/// caller keeps it alive until CC exits.
pub fn write_ca_bundle(ca_pem: &str) -> std::io::Result<tempfile::NamedTempFile> {
    let mut f = tempfile::Builder::new()
        .prefix("claudepot-relay-ca-")
        .suffix(".pem")
        .tempfile()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(f.path(), std::fs::Permissions::from_mode(0o600))?;
    }
    f.write_all(ca_pem.as_bytes())?;
    f.flush()?;
    Ok(f)
}

/// Create the per-upstream profile directory, owner-only on Unix.
pub fn ensure_profile_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Run `binary args…` under `env` with inherited stdio, and return its
/// exit code (`1` when it was killed by a signal).
pub async fn run(binary: &Path, args: &[OsString], env: &LaunchEnv) -> std::io::Result<i32> {
    let mut cmd = tokio::process::Command::new(binary);
    cmd.args(args);
    env.apply(&mut cmd);
    let status = cmd.status().await?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_env::spec::try_spec;

    fn up(s: &str) -> UpstreamId {
        UpstreamId::parse(s).unwrap()
    }

    #[test]
    fn the_cleared_list_comes_from_the_spec_and_covers_every_provider_selector() {
        let spec = try_spec().unwrap();
        let cleared = cleared_vars(spec);
        for must in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_MODEL",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
            "ANTHROPIC_CUSTOM_HEADERS",
        ] {
            assert!(cleared.iter().any(|c| c == must), "{must} not cleared");
        }
        // Every provider-managed variable in the spec is in the list.
        for v in spec.vars.iter().filter(|v| v.safety.provider_managed) {
            assert!(cleared.contains(&v.name), "{} not cleared", v.name);
        }
        // Ordinary settings survive.
        assert!(!cleared.iter().any(|c| c == "CLAUDE_CODE_MAX_OUTPUT_TOKENS"));
    }

    #[test]
    fn a_profile_dir_is_never_a_reserved_windows_name() {
        let base = Path::new("data");
        for id in ["con", "nul", "aux", "prn", "com1", "lpt9", "sub"] {
            let dir = profile_dir(base, &up(id));
            let leaf = dir.file_name().unwrap().to_str().unwrap();
            assert_eq!(leaf, format!("u-{id}"));
            assert_eq!(dir.parent().unwrap(), base.join(PROFILES_DIR));
        }
    }

    /// The child sees exactly the arguments given, and its exit code is
    /// what `run` returns — `claudepot relay run` exits with it.
    #[cfg(unix)]
    #[tokio::test]
    async fn run_forwards_arguments_and_returns_the_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("args");
        let env = LaunchEnv {
            remove: vec![],
            set: vec![],
            token: Secret::new(String::from("cprl_device")),
        };
        let args: Vec<OsString> = vec![
            "-c".into(),
            r#"printf '%s|' "$@" > "$0"; exit 7"#.into(),
            out.as_os_str().into(),
            "-p".into(),
            "two words".into(),
        ];
        let code = run(Path::new("/bin/sh"), &args, &env).await.unwrap();
        assert_eq!(code, 7);
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "-p|two words|");
    }

    #[test]
    fn the_ca_bundle_is_deleted_when_its_handle_drops() {
        let f = write_ca_bundle("PEM").unwrap();
        let path = f.path().to_owned();
        assert!(path.exists());
        drop(f);
        assert!(!path.exists());
    }

    #[test]
    fn the_base_url_always_names_the_upstream() {
        let base = reqwest::Url::parse("https://relay.test:8787/").unwrap();
        assert_eq!(
            base_url_for(&base, &up("sub")),
            "https://relay.test:8787/u/sub"
        );
        let base = reqwest::Url::parse("http://127.0.0.1:8787").unwrap();
        assert_eq!(base_url_for(&base, &up("ds")), "http://127.0.0.1:8787/u/ds");
    }

    #[test]
    fn the_ca_bundle_is_owner_only_and_holds_the_pem() {
        let f = write_ca_bundle("PEM").unwrap();
        assert_eq!(std::fs::read_to_string(f.path()).unwrap(), "PEM");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(f.path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let path = f.path().to_owned();
        drop(f);
        assert!(!path.exists(), "the bundle is deleted with its handle");
    }

    #[test]
    fn debug_does_not_print_the_token() {
        let spec = try_spec().unwrap();
        let env = build(
            spec,
            "https://r/u/sub",
            Secret::new(String::from("cprl_canary")),
            None,
            Path::new("/p"),
        );
        assert!(!format!("{env:?}").contains("canary"));
    }

    const LAUNCH_TEST: &str =
        "relay::launch::tests::the_launch_env_replaces_the_inherited_provider_env";

    /// Runs in a child whose inherited environment carries a competing
    /// provider setup; the launched process must see only the relay's.
    #[test]
    fn the_launch_env_replaces_the_inherited_provider_env() {
        if !crate::testing::in_child(LAUNCH_TEST) {
            crate::testing::run_in_child(
                LAUNCH_TEST,
                &[
                    ("ANTHROPIC_API_KEY", "sk-ant-api03-leak".as_ref()),
                    ("ANTHROPIC_BASE_URL", "https://elsewhere.example".as_ref()),
                    ("CLAUDE_CODE_USE_BEDROCK", "1".as_ref()),
                    ("ANTHROPIC_MODEL", "pinned".as_ref()),
                    (
                        "ANTHROPIC_CUSTOM_HEADERS",
                        "Authorization: Bearer x".as_ref(),
                    ),
                    ("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant-oat01-leak".as_ref()),
                    ("RELAY_TEST_KEEP", "kept".as_ref()),
                ],
                &[],
            );
            return;
        }
        let spec = try_spec().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let profile = profile_dir(dir.path(), &up("sub"));
        ensure_profile_dir(&profile).unwrap();
        let bundle = write_ca_bundle("PEM").unwrap();
        let env = build(
            spec,
            "https://relay.test:8787/u/sub",
            Secret::new(String::from("cprl_device")),
            Some(bundle.path()),
            &profile,
        );

        #[cfg(unix)]
        let (bin, args): (PathBuf, Vec<OsString>) = ("/usr/bin/env".into(), vec![]);
        #[cfg(windows)]
        let (bin, args): (PathBuf, Vec<OsString>) = ("cmd".into(), vec!["/C".into(), "set".into()]);

        let mut cmd = tokio::process::Command::new(&bin);
        cmd.args(&args);
        env.apply(&mut cmd);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let out = rt.block_on(async { cmd.output().await }).unwrap();
        assert!(out.status.success());
        let text = String::from_utf8_lossy(&out.stdout);
        let vars: std::collections::HashMap<&str, &str> =
            text.lines().filter_map(|l| l.split_once('=')).collect();

        assert_eq!(vars.get("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST"), Some(&"1"));
        assert_eq!(
            vars.get("ANTHROPIC_BASE_URL"),
            Some(&"https://relay.test:8787/u/sub")
        );
        assert_eq!(vars.get("ANTHROPIC_AUTH_TOKEN"), Some(&"cprl_device"));
        assert_eq!(
            vars.get("NODE_EXTRA_CA_CERTS").map(PathBuf::from),
            Some(bundle.path().to_owned())
        );
        assert_eq!(
            vars.get("CLAUDE_CONFIG_DIR").map(PathBuf::from),
            Some(profile.clone())
        );
        for gone in [
            "ANTHROPIC_API_KEY",
            "CLAUDE_CODE_USE_BEDROCK",
            "ANTHROPIC_MODEL",
            "ANTHROPIC_CUSTOM_HEADERS",
            "CLAUDE_CODE_OAUTH_TOKEN",
        ] {
            assert!(!vars.contains_key(gone), "{gone} leaked into the launch");
        }
        assert!(!text.contains("leak"));
        assert_eq!(vars.get("RELAY_TEST_KEEP"), Some(&"kept"));
    }
}
