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
use super::types::{
    AuthScheme, BedrockConfig, FoundryConfig, Route, RouteProvider, VertexConfig,
};
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
        RouteProvider::Bedrock(cfg) => render_bedrock(route, cfg),
        RouteProvider::Vertex(cfg) => render_vertex(route, cfg),
        RouteProvider::Foundry(cfg) => render_foundry(route, cfg),
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

fn render_header(route: &Route) -> String {
    let mut out = String::with_capacity(256);
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
    out
}

fn render_bedrock(route: &Route, cfg: &BedrockConfig) -> String {
    let mut out = render_header(route);
    out.push_str("exec env \\\n");
    out.push_str(&kv_line("CLAUDE_CODE_USE_BEDROCK", "1"));
    out.push_str(&kv_line("AWS_REGION", &cfg.region));
    out.push_str(&kv_line("ANTHROPIC_SMALL_FAST_MODEL_AWS_REGION", &cfg.region));
    if let Some(token) = &cfg.bearer_token {
        out.push_str(&kv_line("AWS_BEARER_TOKEN_BEDROCK", token));
    }
    if let Some(profile) = &cfg.aws_profile {
        out.push_str(&kv_line("AWS_PROFILE", profile));
    }
    if let Some(url) = &cfg.base_url {
        out.push_str(&kv_line("ANTHROPIC_BEDROCK_BASE_URL", url));
    }
    if cfg.skip_aws_auth {
        out.push_str(&kv_line("CLAUDE_CODE_SKIP_BEDROCK_AUTH", "1"));
    }
    out.push_str(&kv_line("ANTHROPIC_MODEL", &route.model));
    let small = route.small_fast_model.as_deref().unwrap_or(&route.model);
    out.push_str(&kv_line("ANTHROPIC_SMALL_FAST_MODEL", small));
    out.push_str("  claude \"$@\"\n");
    out
}

fn render_vertex(route: &Route, cfg: &VertexConfig) -> String {
    let mut out = render_header(route);
    out.push_str("exec env \\\n");
    out.push_str(&kv_line("CLAUDE_CODE_USE_VERTEX", "1"));
    out.push_str(&kv_line("ANTHROPIC_VERTEX_PROJECT_ID", &cfg.project_id));
    if let Some(region) = &cfg.region {
        out.push_str(&kv_line("CLOUD_ML_REGION", region));
    }
    if let Some(url) = &cfg.base_url {
        out.push_str(&kv_line("ANTHROPIC_VERTEX_BASE_URL", url));
    }
    if cfg.skip_gcp_auth {
        out.push_str(&kv_line("CLAUDE_CODE_SKIP_VERTEX_AUTH", "1"));
    }
    out.push_str(&kv_line("ANTHROPIC_MODEL", &route.model));
    let small = route.small_fast_model.as_deref().unwrap_or(&route.model);
    out.push_str(&kv_line("ANTHROPIC_SMALL_FAST_MODEL", small));
    out.push_str("  claude \"$@\"\n");
    out
}

fn render_foundry(route: &Route, cfg: &FoundryConfig) -> String {
    let mut out = render_header(route);
    out.push_str("exec env \\\n");
    out.push_str(&kv_line("CLAUDE_CODE_USE_FOUNDRY", "1"));
    if let Some(url) = &cfg.base_url {
        out.push_str(&kv_line("ANTHROPIC_FOUNDRY_BASE_URL", url));
    } else if let Some(resource) = &cfg.resource {
        out.push_str(&kv_line("ANTHROPIC_FOUNDRY_RESOURCE", resource));
    }
    if let Some(key) = &cfg.api_key {
        out.push_str(&kv_line("ANTHROPIC_FOUNDRY_API_KEY", key));
    }
    if cfg.skip_azure_auth {
        out.push_str(&kv_line("CLAUDE_CODE_SKIP_FOUNDRY_AUTH", "1"));
    }
    out.push_str(&kv_line("ANTHROPIC_MODEL", &route.model));
    let small = route.small_fast_model.as_deref().unwrap_or(&route.model);
    out.push_str(&kv_line("ANTHROPIC_SMALL_FAST_MODEL", small));
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
    use crate::routes::types::{
        AuthScheme, BedrockConfig, FoundryConfig, GatewayConfig, Route, RouteProvider,
        VertexConfig,
    };
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

    fn bedrock_sample(model: &str, region: &str) -> Route {
        Route {
            id: Uuid::new_v4(),
            name: "Bedrock prod".into(),
            provider: RouteProvider::Bedrock(BedrockConfig {
                region: region.into(),
                bearer_token: Some("aws-bearer-token".into()),
                base_url: None,
                aws_profile: Some("claudepot-prod".into()),
                skip_aws_auth: false,
            }),
            model: model.into(),
            small_fast_model: None,
            additional_models: vec![],
            wrapper_name: "claude-bedrock".into(),
            deployment_organization_uuid: Uuid::new_v4(),
            active_on_desktop: false,
            installed_on_cli: false,
        }
    }

    fn vertex_sample(model: &str, project: &str) -> Route {
        Route {
            id: Uuid::new_v4(),
            name: "Vertex eu".into(),
            provider: RouteProvider::Vertex(VertexConfig {
                project_id: project.into(),
                region: Some("us-east5".into()),
                base_url: None,
                skip_gcp_auth: false,
            }),
            model: model.into(),
            small_fast_model: None,
            additional_models: vec![],
            wrapper_name: "claude-vertex".into(),
            deployment_organization_uuid: Uuid::new_v4(),
            active_on_desktop: false,
            installed_on_cli: false,
        }
    }

    fn foundry_sample(model: &str, resource_or_url: Result<&str, &str>) -> Route {
        let (base_url, resource) = match resource_or_url {
            Ok(res) => (None, Some(res.to_string())),
            Err(url) => (Some(url.to_string()), None),
        };
        Route {
            id: Uuid::new_v4(),
            name: "Foundry".into(),
            provider: RouteProvider::Foundry(FoundryConfig {
                api_key: Some("foundry-key-123".into()),
                base_url,
                resource,
                skip_azure_auth: false,
            }),
            model: model.into(),
            small_fast_model: None,
            additional_models: vec![],
            wrapper_name: "claude-foundry".into(),
            deployment_organization_uuid: Uuid::new_v4(),
            active_on_desktop: false,
            installed_on_cli: false,
        }
    }

    #[test]
    fn render_bedrock_emits_use_flag_and_region() {
        let r = bedrock_sample(
            "us.anthropic.claude-sonnet-4-20250514-v1:0",
            "us-west-2",
        );
        let s = render_script(&r);
        assert!(s.contains("CLAUDE_CODE_USE_BEDROCK='1'"));
        assert!(s.contains("AWS_REGION='us-west-2'"));
        assert!(s.contains("AWS_BEARER_TOKEN_BEDROCK='aws-bearer-token'"));
        assert!(s.contains("AWS_PROFILE='claudepot-prod'"));
        assert!(s.contains("ANTHROPIC_SMALL_FAST_MODEL_AWS_REGION='us-west-2'"));
        assert!(s
            .contains("ANTHROPIC_MODEL='us.anthropic.claude-sonnet-4-20250514-v1:0'"));
        assert!(s.contains("# provider: bedrock"));
    }

    #[test]
    fn render_bedrock_skip_auth_flag() {
        let mut r = bedrock_sample("anthropic.claude-haiku-4-5", "us-east-1");
        if let RouteProvider::Bedrock(ref mut cfg) = r.provider {
            cfg.skip_aws_auth = true;
        }
        let s = render_script(&r);
        assert!(s.contains("CLAUDE_CODE_SKIP_BEDROCK_AUTH='1'"));
    }

    #[test]
    fn render_vertex_emits_required_keys() {
        let r = vertex_sample("claude-sonnet-4-5@20250929", "my-gcp-proj");
        let s = render_script(&r);
        assert!(s.contains("CLAUDE_CODE_USE_VERTEX='1'"));
        assert!(s.contains("ANTHROPIC_VERTEX_PROJECT_ID='my-gcp-proj'"));
        assert!(s.contains("CLOUD_ML_REGION='us-east5'"));
        assert!(s.contains("ANTHROPIC_MODEL='claude-sonnet-4-5@20250929'"));
        assert!(s.contains("# provider: vertex"));
    }

    #[test]
    fn render_vertex_skip_auth_flag() {
        let mut r = vertex_sample("claude-sonnet-4-5", "p");
        if let RouteProvider::Vertex(ref mut cfg) = r.provider {
            cfg.skip_gcp_auth = true;
        }
        let s = render_script(&r);
        assert!(s.contains("CLAUDE_CODE_SKIP_VERTEX_AUTH='1'"));
    }

    #[test]
    fn render_foundry_resource_form() {
        let r = foundry_sample("claude-sonnet-4-5", Ok("my-resource"));
        let s = render_script(&r);
        assert!(s.contains("CLAUDE_CODE_USE_FOUNDRY='1'"));
        assert!(s.contains("ANTHROPIC_FOUNDRY_RESOURCE='my-resource'"));
        assert!(s.contains("ANTHROPIC_FOUNDRY_API_KEY='foundry-key-123'"));
        assert!(!s.contains("ANTHROPIC_FOUNDRY_BASE_URL"));
    }

    #[test]
    fn render_foundry_base_url_form() {
        let r = foundry_sample(
            "claude-sonnet-4-5",
            Err("https://my-resource.openai.azure.com"),
        );
        let s = render_script(&r);
        assert!(s.contains(
            "ANTHROPIC_FOUNDRY_BASE_URL='https://my-resource.openai.azure.com'"
        ));
        assert!(!s.contains("ANTHROPIC_FOUNDRY_RESOURCE"));
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
