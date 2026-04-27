//! CLI wrapper-script writer. Each route, when "installed on CLI",
//! materializes as `~/.claudepot/bin/<name>` — a `/bin/sh` script
//! that sets the right env vars and execs the real `claude`.
//!
//! On Unix the script is created mode 0700. On Windows it'd be a
//! `.cmd` file (out of scope for phase-1 MVP).

use std::path::{Path, PathBuf};

use crate::fs_utils;
use crate::paths::claudepot_data_dir;

use super::error::RouteError;
use super::slug::sanitize_wrapper_name;
use super::types::{AuthScheme, Route, RouteProvider};
use super::CLAUDEPOT_MANAGED_MARKER;

/// `~/.claudepot/bin/`.
pub fn wrapper_dir() -> PathBuf {
    claudepot_data_dir().join("bin")
}

/// Full path to the wrapper for a given (already sanitized) name.
pub fn wrapper_path(name: &str) -> PathBuf {
    wrapper_dir().join(name)
}

/// Materialize a route as a wrapper script. Returns the absolute
/// path that was written.
pub fn write_wrapper(route: &Route) -> Result<PathBuf, RouteError> {
    let name = sanitize_wrapper_name(&route.wrapper_name)
        .map_err(|e| RouteError::InvalidWrapperName(route.wrapper_name.clone(), e.to_string()))?;
    if name == "claude" {
        return Err(RouteError::WrapperShadowsClaude(name));
    }
    let path = wrapper_path(&name);
    let script = render_script(route);
    fs_utils::atomic_write(&path, script.as_bytes())?;
    set_executable(&path)?;
    Ok(path)
}

/// Remove a wrapper script. Idempotent — missing file is not an error.
pub fn delete_wrapper(name: &str) -> Result<(), RouteError> {
    // Skip sanitization here (we delete by stored name verbatim) but
    // refuse to follow `..` or absolute paths that would escape
    // the bin dir.
    if name.contains('/') || name.contains('\\') || name.starts_with('.') {
        return Err(RouteError::InvalidWrapperName(
            name.to_string(),
            "name must not contain path separators or leading dot".into(),
        ));
    }
    let path = wrapper_path(name);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(RouteError::Io(e)),
    }
}

fn render_script(route: &Route) -> String {
    match &route.provider {
        RouteProvider::Gateway(cfg) => render_gateway(route, cfg),
    }
}

fn render_gateway(route: &Route, cfg: &super::types::GatewayConfig) -> String {
    let mut out = String::with_capacity(512);
    out.push_str("#!/bin/sh\n");
    out.push_str("# claudepot-managed wrapper — third-party LLM route\n");
    out.push_str(&format!("# {}: true\n", CLAUDEPOT_MANAGED_MARKER));
    out.push_str(&format!("# route: {}\n", shell_comment_safe(&route.name)));
    out.push_str(&format!(
        "# provider: {}\n",
        route.provider.kind().as_str()
    ));
    out.push_str("#\n");
    out.push_str("# Edit via Claudepot's Third-party section, not by hand —\n");
    out.push_str("# subsequent route updates will overwrite this file.\n");
    out.push_str("\n");
    out.push_str("exec env \\\n");
    out.push_str(&kv_line("ANTHROPIC_BASE_URL", &cfg.base_url));
    out.push_str(&kv_line("ANTHROPIC_AUTH_TOKEN", &cfg.api_key));
    if cfg.auth_scheme == AuthScheme::Bearer {
        // CC's default is bearer; only emit a hint if the user picks
        // a non-default scheme. For now we pass the key as-is and let
        // CC's standard `Authorization: Bearer …` header carry it.
    }
    out.push_str(&kv_line("ANTHROPIC_MODEL", &route.model));
    let small = route.small_fast_model.as_deref().unwrap_or(&route.model);
    out.push_str(&kv_line("ANTHROPIC_SMALL_FAST_MODEL", small));
    if cfg.enable_tool_search {
        out.push_str(&kv_line("ENABLE_TOOL_SEARCH", "true"));
    }
    out.push_str("  claude \"$@\"\n");
    out
}

/// `KEY="value" \` with the value shell-escaped.
fn kv_line(k: &str, v: &str) -> String {
    format!("  {k}={} \\\n", shell_quote(v))
}

/// Single-quote-wrap with embedded-quote escaping (POSIX-safe).
fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            // Close, escaped quote, reopen.
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// For comments — strip newlines that could break the comment block.
fn shell_comment_safe(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
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
    // Windows: chmod doesn't apply. The .cmd extension carries
    // executability at the shell level. Phase-1 MVP is Unix-first;
    // Windows wrappers land later.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::types::{AuthScheme, GatewayConfig, Route, RouteProvider};
    use uuid::Uuid;

    fn sample(model: &str, key: &str, name: &str) -> Route {
        Route {
            id: Uuid::new_v4(),
            name: format!("Test {name}"),
            provider: RouteProvider::Gateway(GatewayConfig {
                base_url: "http://127.0.0.1:11434".into(),
                api_key: key.into(),
                auth_scheme: AuthScheme::Bearer,
                enable_tool_search: false,
            }),
            model: model.into(),
            small_fast_model: None,
            additional_models: vec![],
            wrapper_name: name.into(),
            deployment_organization_uuid: Uuid::new_v4(),
            active_on_desktop: false,
            installed_on_cli: false,
        }
    }

    #[test]
    fn shell_quote_no_special() {
        assert_eq!(shell_quote("foo"), "'foo'");
    }

    #[test]
    fn shell_quote_with_quote() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn shell_quote_with_dollar() {
        assert_eq!(shell_quote("$HOME"), "'$HOME'");
    }

    #[test]
    fn render_gateway_includes_marker_and_env() {
        let r = sample("llama3.2:3b", "ollama", "claude-llama3");
        let s = render_script(&r);
        assert!(s.starts_with("#!/bin/sh"));
        assert!(s.contains("claudepot_managed: true"));
        assert!(s.contains("ANTHROPIC_BASE_URL='http://127.0.0.1:11434'"));
        assert!(s.contains("ANTHROPIC_AUTH_TOKEN='ollama'"));
        assert!(s.contains("ANTHROPIC_MODEL='llama3.2:3b'"));
        assert!(s.contains("ANTHROPIC_SMALL_FAST_MODEL='llama3.2:3b'"));
        assert!(s.trim_end().ends_with("claude \"$@\""));
    }

    #[test]
    fn render_gateway_with_distinct_small_fast() {
        let mut r = sample("llama3.2:8b", "ollama", "claude-llama3");
        r.small_fast_model = Some("llama3.2:3b".into());
        let s = render_script(&r);
        assert!(s.contains("ANTHROPIC_MODEL='llama3.2:8b'"));
        assert!(s.contains("ANTHROPIC_SMALL_FAST_MODEL='llama3.2:3b'"));
    }

    #[test]
    fn render_gateway_with_tool_search() {
        let mut r = sample("llama3.2:3b", "ollama", "claude-llama3");
        if let RouteProvider::Gateway(ref mut cfg) = r.provider {
            cfg.enable_tool_search = true;
        }
        let s = render_script(&r);
        assert!(s.contains("ENABLE_TOOL_SEARCH='true'"));
    }

    #[test]
    fn render_gateway_without_tool_search_omits_var() {
        let r = sample("llama3.2:3b", "ollama", "claude-llama3");
        let s = render_script(&r);
        assert!(!s.contains("ENABLE_TOOL_SEARCH"));
    }

    #[test]
    fn render_gateway_quotes_keys_with_quotes() {
        let r = sample("llama", "weird'key", "claude-x");
        let s = render_script(&r);
        // Must round-trip safely through sh -n.
        assert!(s.contains("'weird'\\''key'"));
    }

    #[test]
    fn delete_wrapper_rejects_path_traversal() {
        assert!(delete_wrapper("../etc/passwd").is_err());
        assert!(delete_wrapper(".bashrc").is_err());
        assert!(delete_wrapper("foo/bar").is_err());
    }

    #[test]
    fn delete_wrapper_missing_is_ok() {
        // Even if the data dir doesn't have the file, deletion is
        // idempotent.
        let result = delete_wrapper("nonexistent-claudepot-wrapper-xyz");
        assert!(result.is_ok());
    }
}
