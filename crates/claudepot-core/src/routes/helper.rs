//! Credential-helper script generation. When a route's secret lives
//! in the OS keychain (see `keychain.rs`), the wrapper and Cowork-on-3P
//! configs both invoke a tiny helper script that prints the secret
//! to stdout. The script is plain `/bin/sh` and runs `security
//! find-generic-password -s claudepot-routes -a <account> -w` on
//! macOS. Linux/Windows variants land alongside platform support.
//!
//! Path layout:
//!   `~/.claudepot/bin/.helpers/<route-id>-<field>.sh`
//!
//! Mode `0700` (owner-only) — the script doesn't carry the secret
//! itself, but it does describe how to retrieve it, and any leak
//! gives an attacker on the same box one less unknown.

use std::path::{Path, PathBuf};

use crate::fs_utils;
use crate::paths::claudepot_data_dir;

use super::error::RouteError;
use super::keychain::SecretField;
use super::types::RouteId;
use super::CLAUDEPOT_MANAGED_MARKER;

/// `~/.claudepot/bin/.helpers/`.
pub fn helpers_dir() -> PathBuf {
    claudepot_data_dir().join("bin").join(".helpers")
}

/// `~/.claudepot/bin/.helpers/<route-id>-<field>.sh`.
pub fn helper_path(route_id: RouteId, field: SecretField) -> PathBuf {
    helpers_dir().join(format!("{}-{}.sh", route_id, field_suffix(field)))
}

/// Materialize the helper script. Returns the absolute path
/// written. Phase-1 keychain-mode is macOS-only; non-macOS hosts
/// are rejected so a route can't persist `use_keychain: true` while
/// no working helper actually exists.
pub fn write_helper(
    route_id: RouteId,
    field: SecretField,
) -> Result<PathBuf, RouteError> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = route_id;
        let _ = field;
        return Err(RouteError::UnsupportedPlatform(
            "OS-keychain-backed routes are currently macOS-only",
        ));
    }
    #[cfg(target_os = "macos")]
    {
        let path = helper_path(route_id, field);
        let body = render_helper(route_id, field);
        fs_utils::atomic_write(&path, body.as_bytes())?;
        set_executable(&path)?;
        Ok(path)
    }
}

/// Best-effort cleanup of helpers for a given route. `None` field
/// means "remove every helper for this route."
pub fn delete_helpers(
    route_id: RouteId,
    field: Option<SecretField>,
) -> Result<(), RouteError> {
    let targets: Vec<SecretField> = match field {
        Some(f) => vec![f],
        None => vec![
            SecretField::GatewayApiKey,
            SecretField::BedrockBearerToken,
            SecretField::FoundryApiKey,
        ],
    };
    for f in targets {
        let p = helper_path(route_id, f);
        match std::fs::remove_file(&p) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(RouteError::Io(e)),
        }
    }
    Ok(())
}

fn field_suffix(field: SecretField) -> &'static str {
    match field {
        SecretField::GatewayApiKey => "api_key",
        SecretField::BedrockBearerToken => "bearer_token",
        SecretField::FoundryApiKey => "foundry_api_key",
    }
}

#[cfg(target_os = "macos")]
fn render_helper(route_id: RouteId, field: SecretField) -> String {
    let acct = format!("{}-{}", route_id, field_suffix(field));
    let mut s = String::with_capacity(256);
    s.push_str("#!/bin/sh\n");
    s.push_str("# claudepot-managed credential helper\n");
    s.push_str(&format!("# {}: true\n", CLAUDEPOT_MANAGED_MARKER));
    s.push_str(&format!("# route: {}\n", route_id));
    s.push_str(&format!("# field: {}\n", field_suffix(field)));
    s.push_str(
        "# Reads from macOS keychain item written by Claudepot — never echoes.\n",
    );
    s.push_str(
        "# Used by the wrapper script (\\$()) and by Cowork on 3P's inferenceCredentialHelper.\n",
    );
    s.push('\n');
    s.push_str(&format!(
        "exec /usr/bin/security find-generic-password -s claudepot-routes -a {} -w\n",
        shell_quote(&acct)
    ));
    s
}

#[cfg(not(target_os = "macos"))]
fn render_helper(route_id: RouteId, field: SecretField) -> String {
    let mut s = String::new();
    s.push_str("#!/bin/sh\n");
    s.push_str("# claudepot-managed credential helper (stub)\n");
    s.push_str(&format!("# {}: true\n", CLAUDEPOT_MANAGED_MARKER));
    s.push_str(&format!("# route: {}\n", route_id));
    s.push_str(&format!("# field: {}\n", field_suffix(field)));
    s.push_str(
        "echo 'claudepot: keychain helper not implemented on this platform' >&2\n",
    );
    s.push_str("exit 1\n");
    s
}

/// Shell-safe single-quote of a value embedded in the helper script.
fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), RouteError> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o700);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), RouteError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn helper_path_uses_field_suffix() {
        let id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let p = helper_path(id, SecretField::GatewayApiKey);
        assert!(p.ends_with("11111111-1111-1111-1111-111111111111-api_key.sh"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_helper_invokes_security_with_account() {
        let id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let s = render_helper(id, SecretField::GatewayApiKey);
        assert!(s.starts_with("#!/bin/sh"));
        assert!(s.contains(
            "/usr/bin/security find-generic-password -s claudepot-routes -a '11111111-1111-1111-1111-111111111111-api_key' -w"
        ));
        assert!(s.contains("# claudepot_managed: true"));
    }

    #[test]
    fn shell_quote_simple() {
        assert_eq!(shell_quote("foo-bar"), "'foo-bar'");
    }

    #[test]
    fn shell_quote_with_quote() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}
