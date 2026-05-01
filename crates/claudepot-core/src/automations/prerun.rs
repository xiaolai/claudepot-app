//! Pre-run gate for template-driven automations.
//!
//! This is the Rust-side gate the shim invokes before
//! `claude -p`. It resolves the assigned route, probes its
//! reachability, and applies the blueprint's `fallback_policy`
//! and `privacy` constraints.
//!
//! The gate's output is a single `RouteDecision` — either a
//! green light to run on a wrapper, a fallback to a different
//! wrapper, or a skip with a recorded reason.
//!
//! See `dev-docs/templates-implementation-plan.md` §5.3.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::routes::{Route, RouteId, RouteProvider};

/// What the gate decided. The shim writes this to
/// `<run-dir>/prerun-decision.json`; `record_run` merges it into
/// `AutomationRun.route_decision`.
///
/// This mirrors `automations::types::RouteDecision` but lives
/// here so the prerun module doesn't have a circular import.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PrerunDecision {
    /// Run on the named route (or the user's primary `claude`
    /// when route_id is None).
    Ran {
        route_id: Option<String>,
        wrapper_name: Option<String>,
    },
    /// Assigned route was unreachable; fell back to the user's
    /// primary `claude`. Privacy must allow it.
    Fallback {
        from: String,
        to_wrapper: Option<String>,
        reason: String,
    },
    /// Run was skipped (route unreachable + policy = skip,
    /// or invalid route).
    Skipped { reason: String },
    /// Skipped + the caller should post a notification.
    SkippedAlerted { reason: String },
}

/// Provider-specific probe configuration. Authors of this
/// module can add new provider arms as Anthropic extends the
/// route surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeKind {
    /// HTTP HEAD on the gateway base URL. Used for Ollama,
    /// OpenRouter, hosted LiteLLM, etc.
    HttpHead,
    /// `GET /api/tags` — Ollama-specific liveness check.
    OllamaTags,
    /// No probe — defer to the SDK's own error handling at run
    /// time. Cloud-IAM providers (Bedrock, Vertex, Foundry)
    /// take this path.
    None,
}

/// Choose a probe shape from a route's provider.
pub fn probe_for(route: &Route) -> ProbeKind {
    match &route.provider {
        RouteProvider::Gateway(cfg) => {
            if cfg.base_url.contains("/api/")
                || cfg.base_url.contains("11434") /* default Ollama port */
            {
                ProbeKind::OllamaTags
            } else {
                ProbeKind::HttpHead
            }
        }
        RouteProvider::Bedrock(_) | RouteProvider::Vertex(_) | RouteProvider::Foundry(_) => {
            ProbeKind::None
        }
    }
}

/// Synchronous probe. Returns `Ok(())` when the route is
/// reachable, `Err(String)` with a human-readable reason when
/// not. Probe duration is capped at `timeout`.
pub fn probe_sync(route: &Route, timeout: Duration) -> Result<(), String> {
    let kind = probe_for(route);
    if matches!(kind, ProbeKind::None) {
        // No pre-probe; treat as reachable. Run-time SDK errors
        // will surface through claude -p's own error path.
        return Ok(());
    }

    let base = match &route.provider {
        RouteProvider::Gateway(cfg) => cfg.base_url.clone(),
        _ => return Ok(()), // unreachable — already handled by ProbeKind::None
    };

    let url = match kind {
        ProbeKind::OllamaTags => format!("{}/api/tags", base.trim_end_matches('/')),
        ProbeKind::HttpHead | ProbeKind::None => base,
    };

    // The workspace's reqwest doesn't enable `blocking` (we
    // already pull tokio in for the rest of claudepot-core).
    // Build a current-thread runtime per-invocation: the gate
    // is single-shot inside a CLI process, so a tiny runtime
    // here is cheaper than enabling another reqwest feature.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime build: {e}"))?;

    runtime.block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| format!("reqwest client build: {e}"))?;

        let resp: Result<reqwest::Response, reqwest::Error> = match kind {
            ProbeKind::OllamaTags => client.get(&url).send().await,
            _ => client.head(&url).send().await,
        };

        match resp {
            Ok(r) if r.status().is_success() || r.status().is_redirection() => Ok(()),
            Ok(r) => Err(format!("probe got {} from {url}", r.status())),
            Err(e) => Err(format!("probe failed for {url}: {e}")),
        }
    })
}

/// Inputs the gate needs from the surrounding automation. The
/// caller (CLI subcommand) materializes these from
/// `AutomationStore` + `RouteStore` + `TemplateRegistry`.
#[derive(Debug, Clone)]
pub struct PrerunInput<'a> {
    pub assigned_route: Option<&'a Route>,
    pub default_wrapper: Option<&'a str>,
    /// Templates' privacy class as a string from the blueprint.
    /// `"local"`, `"private-cloud"`, or `"any"`. Non-template
    /// automations set this to `"any"` so fallback to the
    /// default route is always permissible.
    pub privacy: &'a str,
    /// Templates' fallback policy as a string from the
    /// blueprint. `"skip"`, `"use_default_route"`, or
    /// `"alert"`. Defaults to `"skip"` for non-template
    /// automations.
    pub fallback_policy: &'a str,
    /// Probe timeout. CLI default is 3 seconds.
    pub probe_timeout: Duration,
}

/// Resolve the route decision. Pure besides the network probe
/// it issues — no filesystem mutations.
pub fn resolve(input: PrerunInput<'_>) -> PrerunDecision {
    // No assigned route: run on default. Gate is a passthrough.
    let Some(route) = input.assigned_route else {
        return PrerunDecision::Ran {
            route_id: None,
            wrapper_name: input.default_wrapper.map(String::from),
        };
    };

    match probe_sync(route, input.probe_timeout) {
        Ok(()) => PrerunDecision::Ran {
            route_id: Some(route.id.to_string()),
            wrapper_name: Some(route.wrapper_name.clone()),
        },
        Err(reason) => apply_fallback_policy(route.id, input, reason),
    }
}

fn apply_fallback_policy(
    failed: RouteId,
    input: PrerunInput<'_>,
    reason: String,
) -> PrerunDecision {
    // local privacy + any policy → skip with alert (cloud
    // fallback would violate the privacy contract).
    if input.privacy == "local" {
        return PrerunDecision::SkippedAlerted {
            reason: format!("local route {failed} unreachable: {reason}"),
        };
    }
    match input.fallback_policy {
        "use_default_route" => PrerunDecision::Fallback {
            from: failed.to_string(),
            to_wrapper: input.default_wrapper.map(String::from),
            reason: format!("route {failed} unreachable; using default: {reason}"),
        },
        "alert" => PrerunDecision::SkippedAlerted {
            reason: format!("route {failed} unreachable: {reason}"),
        },
        // "skip" or anything we don't recognize: silent skip.
        _ => PrerunDecision::Skipped {
            reason: format!("route {failed} unreachable: {reason}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::{
        AuthScheme, BedrockConfig, GatewayConfig, ProviderKind, Route, RouteProvider,
    };
    use uuid::Uuid;

    fn route_local() -> Route {
        Route {
            id: Uuid::new_v4(),
            name: "Local Ollama".into(),
            provider: RouteProvider::Gateway(GatewayConfig {
                base_url: "http://127.0.0.1:11434".into(),
                api_key: "ollama".into(),
                auth_scheme: AuthScheme::Bearer,
                enable_tool_search: false,
                use_keychain: false,
            }),
            model: "llama3.2:3b".into(),
            small_fast_model: None,
            additional_models: vec![],
            wrapper_name: "claude-llama3-2-3b".into(),
            deployment_organization_uuid: Uuid::new_v4(),
            active_on_desktop: false,
            installed_on_cli: true,
            is_private_cloud: false,
            capabilities_override: None,
        }
    }

    fn route_bedrock() -> Route {
        Route {
            id: Uuid::new_v4(),
            name: "Corp Bedrock".into(),
            provider: RouteProvider::Bedrock(BedrockConfig {
                region: "us-east-1".into(),
                bearer_token: None,
                base_url: None,
                aws_profile: Some("corp".into()),
                skip_aws_auth: false,
                use_keychain: false,
            }),
            model: "anthropic.claude-3-5-sonnet-20240620-v1:0".into(),
            small_fast_model: None,
            additional_models: vec![],
            wrapper_name: "claude-bedrock-corp".into(),
            deployment_organization_uuid: Uuid::new_v4(),
            active_on_desktop: false,
            installed_on_cli: true,
            is_private_cloud: true,
            capabilities_override: None,
        }
    }

    #[test]
    fn no_assigned_route_runs_default() {
        let dec = resolve(PrerunInput {
            assigned_route: None,
            default_wrapper: Some("claude"),
            privacy: "any",
            fallback_policy: "skip",
            probe_timeout: Duration::from_millis(50),
        });
        match dec {
            PrerunDecision::Ran {
                route_id,
                wrapper_name,
            } => {
                assert!(route_id.is_none());
                assert_eq!(wrapper_name.as_deref(), Some("claude"));
            }
            other => panic!("expected Ran, got {other:?}"),
        }
    }

    #[test]
    fn local_privacy_with_unreachable_local_route_skips_alerted() {
        // We use an obviously-unreachable port to make the
        // probe fail fast inside the timeout.
        let mut r = route_local();
        if let RouteProvider::Gateway(cfg) = &mut r.provider {
            cfg.base_url = "http://127.0.0.1:1".into();
        }
        let dec = resolve(PrerunInput {
            assigned_route: Some(&r),
            default_wrapper: Some("claude"),
            privacy: "local",
            fallback_policy: "skip",
            probe_timeout: Duration::from_millis(50),
        });
        assert!(
            matches!(dec, PrerunDecision::SkippedAlerted { .. }),
            "got {dec:?}"
        );
    }

    #[test]
    fn any_privacy_with_unreachable_route_and_fallback_policy_falls_back() {
        let mut r = route_local();
        if let RouteProvider::Gateway(cfg) = &mut r.provider {
            cfg.base_url = "http://127.0.0.1:1".into();
        }
        let dec = resolve(PrerunInput {
            assigned_route: Some(&r),
            default_wrapper: Some("claude"),
            privacy: "any",
            fallback_policy: "use_default_route",
            probe_timeout: Duration::from_millis(50),
        });
        match dec {
            PrerunDecision::Fallback { to_wrapper, .. } => {
                assert_eq!(to_wrapper.as_deref(), Some("claude"));
            }
            other => panic!("expected Fallback, got {other:?}"),
        }
    }

    #[test]
    fn any_privacy_with_unreachable_route_and_skip_policy_skips() {
        let mut r = route_local();
        if let RouteProvider::Gateway(cfg) = &mut r.provider {
            cfg.base_url = "http://127.0.0.1:1".into();
        }
        let dec = resolve(PrerunInput {
            assigned_route: Some(&r),
            default_wrapper: Some("claude"),
            privacy: "any",
            fallback_policy: "skip",
            probe_timeout: Duration::from_millis(50),
        });
        assert!(matches!(dec, PrerunDecision::Skipped { .. }), "got {dec:?}");
    }

    #[test]
    fn bedrock_route_skips_probe_and_runs() {
        let r = route_bedrock();
        let dec = resolve(PrerunInput {
            assigned_route: Some(&r),
            default_wrapper: Some("claude"),
            privacy: "any",
            fallback_policy: "skip",
            probe_timeout: Duration::from_millis(50),
        });
        match dec {
            PrerunDecision::Ran { route_id, .. } => {
                assert!(route_id.is_some());
            }
            other => panic!(
                "expected Ran (Bedrock skips probe), got {other:?}; provider kind = {:?}",
                r.provider.kind(),
            ),
        }
        // Reference ProviderKind to silence unused-import noise
        // when this test file ships before downstream usage.
        let _ = ProviderKind::Bedrock;
    }

    #[test]
    fn probe_for_picks_ollama_for_11434() {
        let r = route_local();
        assert_eq!(probe_for(&r), ProbeKind::OllamaTags);
    }

    #[test]
    fn probe_for_picks_none_for_cloud_iam() {
        let r = route_bedrock();
        assert_eq!(probe_for(&r), ProbeKind::None);
    }
}
