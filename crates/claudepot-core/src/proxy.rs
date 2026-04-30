//! Outbound proxy detection for HTTPS requests.
//!
//! Priority order:
//! 1. Environment variables — `HTTPS_PROXY`, `https_proxy`, `ALL_PROXY`,
//!    `all_proxy` (first non-empty value wins). Works when the app is
//!    launched from a shell that already has the vars set (e.g. `pnpm tauri dev`).
//! 2. macOS `SystemConfiguration` framework — `SCDynamicStoreCopyProxies` via
//!    `SCDynamicStore::get_proxies()`. Covers the Finder/Dock launch path where
//!    shell env is absent.
//! 3. No proxy.
//!
//! `NO_PROXY`/`no_proxy` exclusions are honoured at every level: env-var
//! path reads them from the environment; macOS path translates the system
//! `ExceptionsList` to the same comma-separated format.

/// Where the proxy configuration came from — reported by `doctor` so
/// support tickets are diagnosable without guessing the launch context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxySource {
    /// A shell environment variable (name shown, e.g. `"https_proxy"`).
    EnvVar(String),
    /// macOS `SystemConfiguration` framework (`SCDynamicStoreCopyProxies`).
    #[cfg(target_os = "macos")]
    MacosSystem,
    /// No proxy configured anywhere.
    None,
}

impl std::fmt::Display for ProxySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProxySource::EnvVar(name) => write!(f, "env-var: {name}"),
            #[cfg(target_os = "macos")]
            ProxySource::MacosSystem => write!(f, "SystemConfiguration"),
            ProxySource::None => write!(f, "none"),
        }
    }
}

/// Resolved proxy configuration ready to pass to a `reqwest::ClientBuilder`.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Proxy URL (e.g. `http://127.0.0.1:6152`), or `None` if no proxy.
    pub url: Option<String>,
    /// Comma-separated exclusion list for `reqwest::NoProxy::from_string`,
    /// or `None` if there are no exclusions.
    pub no_proxy: Option<String>,
    /// Diagnostic: which detection path produced this result.
    pub source: ProxySource,
}

/// Detect the proxy configuration for outbound HTTPS requests.
///
/// See module-level docs for the priority order.
pub fn detect() -> ProxyConfig {
    // 1. Environment variables
    let env_proxy = ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
        .iter()
        .find_map(|var| {
            let val = std::env::var(var).ok()?;
            if val.is_empty() {
                None
            } else {
                Some((var.to_string(), val))
            }
        });

    if let Some((var_name, url)) = env_proxy {
        return ProxyConfig {
            url: Some(url),
            no_proxy: no_proxy_from_env(),
            source: ProxySource::EnvVar(var_name),
        };
    }

    // 2. macOS system proxy via SystemConfiguration
    #[cfg(target_os = "macos")]
    if let Some((url, no_proxy)) = macos_system_proxy() {
        return ProxyConfig {
            url: Some(url),
            no_proxy,
            source: ProxySource::MacosSystem,
        };
    }

    // 3. No proxy
    ProxyConfig {
        url: None,
        no_proxy: None,
        source: ProxySource::None,
    }
}

/// Apply a `ProxyConfig` to a `reqwest::ClientBuilder`.
///
/// Does nothing if `config.url` is `None`. If the URL is present but
/// unparseable as a proxy URI, the builder is returned unchanged (silent
/// degradation — the request will attempt a direct connection).
pub fn apply(builder: reqwest::ClientBuilder, config: &ProxyConfig) -> reqwest::ClientBuilder {
    let Some(ref url) = config.url else {
        return builder;
    };
    let Ok(proxy) = reqwest::Proxy::all(url) else {
        return builder;
    };
    let no_proxy = config
        .no_proxy
        .as_deref()
        .and_then(reqwest::NoProxy::from_string)
        .or_else(reqwest::NoProxy::from_env);
    builder.proxy(proxy.no_proxy(no_proxy))
}

fn no_proxy_from_env() -> Option<String> {
    ["NO_PROXY", "no_proxy"].iter().find_map(|var| {
        let val = std::env::var(var).ok()?;
        if val.is_empty() {
            None
        } else {
            Some(val)
        }
    })
}

// ─── macOS system proxy via SystemConfiguration ──────────────────────────────

/// Read the HTTPS proxy from the macOS `SystemConfiguration` framework using
/// `SCDynamicStoreCopyProxies`. Returns `(proxy_url, no_proxy_list)` or
/// `None` if HTTPS proxy is not enabled.
#[cfg(target_os = "macos")]
fn macos_system_proxy() -> Option<(String, Option<String>)> {
    use system_configuration::{
        core_foundation::{
            array::CFArray,
            base::TCFType,
            number::CFNumber,
            string::CFString,
        },
        dynamic_store::SCDynamicStoreBuilder,
    };

    let store = SCDynamicStoreBuilder::new("claudepot").build();
    let dict = store.get_proxies()?;

    // Helper: look up a key and downcast to CFNumber → i64
    let get_num = |key: &str| -> Option<i64> {
        let cf_key = CFString::new(key);
        let val = dict.find(&cf_key)?;
        val.downcast::<CFNumber>()?.to_i64()
    };

    // Helper: look up a key and downcast to CFString → String
    let get_str = |key: &str| -> Option<String> {
        let cf_key = CFString::new(key);
        let val = dict.find(&cf_key)?;
        Some(val.downcast::<CFString>()?.to_string())
    };

    // Check HTTPSEnable = 1
    if get_num("HTTPSEnable").unwrap_or(0) != 1 {
        return None;
    }

    let host = get_str("HTTPSProxy")?;
    let port = get_num("HTTPSPort").filter(|&p| p > 0)?;

    if host.is_empty() {
        return None;
    }

    let proxy_url = format!("http://{}:{}", host, port);

    // ExceptionsList → comma-separated no_proxy string.
    // CFArray<CFString> does not implement ConcreteCFType; downcast to the
    // untyped CFArray<*const c_void> and re-wrap each element as CFString.
    let no_proxy = {
        use std::ffi::c_void;
        let exc_key = CFString::new("ExceptionsList");
        dict.find(&exc_key).and_then(|val| {
            let arr = val.downcast::<CFArray<*const c_void>>()?;
            let items: Vec<String> = arr
                .iter()
                .filter_map(|ptr| {
                    if ptr.is_null() {
                        return None;
                    }
                    // SAFETY: every element in ExceptionsList is a CFString.
                    let s: CFString = unsafe { TCFType::wrap_under_get_rule(*ptr as _) };
                    Some(s.to_string())
                })
                .collect();
            if items.is_empty() {
                None
            } else {
                Some(items.join(","))
            }
        })
    };

    Some((proxy_url, no_proxy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_does_not_panic() {
        let _ = detect();
    }

    #[test]
    fn test_proxy_source_display() {
        assert_eq!(ProxySource::None.to_string(), "none");
        assert_eq!(
            ProxySource::EnvVar("https_proxy".into()).to_string(),
            "env-var: https_proxy"
        );
        #[cfg(target_os = "macos")]
        assert_eq!(ProxySource::MacosSystem.to_string(), "SystemConfiguration");
    }
}
