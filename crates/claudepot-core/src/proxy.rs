//! Outbound proxy detection for HTTPS requests.
//!
//! Priority order:
//! 1. Environment variables — `HTTPS_PROXY`, `https_proxy`, `ALL_PROXY`,
//!    `all_proxy` (first non-empty AND parseable value wins). A set-but-
//!    malformed env value falls through to the next source instead of
//!    silently disabling the proxy.
//! 2. macOS `SystemConfiguration` framework — `SCDynamicStoreCopyProxies` via
//!    `SCDynamicStore::get_proxies()`. Covers the Finder/Dock launch path where
//!    shell env is absent. Tries HTTPS first, then SOCKS, then surfaces a
//!    PAC-unsupported diagnostic (without evaluating the script).
//! 3. No proxy.
//!
//! `NO_PROXY`/`no_proxy` exclusions are computed once at detect time and
//! always travel with the resolved `ProxyConfig` — `apply()` does no env
//! fallback. This guarantees the cache key in `oauth::http_client()`
//! reflects every input that affects the built `reqwest::Client`.
//!
//! PAC URL handling is intentionally diagnostic-only: evaluating
//! `FindProxyForURL` requires shipping a JS engine, which is out of scope
//! for this surface. When PAC is detected we surface
//! `ProxySource::MacosPacUnsupported` so `doctor` explains why traffic is
//! still going direct, instead of failing silently. The PAC URL is
//! redacted (userinfo stripped) before display so it is safe to print
//! and emit in JSON.

/// Where the proxy configuration came from — reported by `doctor` so
/// support tickets are diagnosable without guessing the launch context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxySource {
    /// A shell environment variable (name shown, e.g. `"https_proxy"`).
    EnvVar(String),
    /// macOS `SystemConfiguration` HTTPS proxy.
    #[cfg(target_os = "macos")]
    MacosSystemHttps,
    /// macOS `SystemConfiguration` SOCKS proxy (used when HTTPS isn't set).
    #[cfg(target_os = "macos")]
    MacosSystemSocks,
    /// macOS `SystemConfiguration` reports a PAC URL but Claudepot does
    /// not evaluate PAC scripts — traffic goes direct. The string is the
    /// PAC URL itself with userinfo redacted, surfaced for diagnostics.
    #[cfg(target_os = "macos")]
    MacosPacUnsupported(String),
    /// No proxy configured anywhere.
    None,
}

impl ProxySource {
    /// True when the proxy state warrants a warning indicator in the UI.
    /// PAC-unsupported is the only state where the user has *configured*
    /// a proxy but Claudepot cannot honour it — traffic goes direct.
    pub fn is_warning(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            matches!(self, ProxySource::MacosPacUnsupported(_))
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
}

impl std::fmt::Display for ProxySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProxySource::EnvVar(name) => write!(f, "env-var: {name}"),
            #[cfg(target_os = "macos")]
            ProxySource::MacosSystemHttps => write!(f, "SystemConfiguration (HTTPS)"),
            #[cfg(target_os = "macos")]
            ProxySource::MacosSystemSocks => write!(f, "SystemConfiguration (SOCKS)"),
            #[cfg(target_os = "macos")]
            ProxySource::MacosPacUnsupported(url) => {
                write!(
                    f,
                    "PAC configured but unsupported: {} (going direct)",
                    redact_proxy_userinfo(url)
                )
            }
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
    /// or `None` if there are no exclusions. Always reflects the effective
    /// value after merging env `NO_PROXY` — `apply()` does not re-read env.
    pub no_proxy: Option<String>,
    /// Diagnostic: which detection path produced this result.
    pub source: ProxySource,
}

/// Detect the proxy configuration for outbound HTTPS requests.
///
/// See module-level docs for the priority order.
pub fn detect() -> ProxyConfig {
    // 1. Environment variables — first parseable, non-empty value wins.
    //    A set-but-malformed value falls through (e.g. `HTTPS_PROXY=garbage`
    //    no longer disables fallback to the macOS system proxy).
    let env_proxy = ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
        .iter()
        .find_map(|var| {
            let val = std::env::var(var).ok()?;
            if val.is_empty() {
                return None;
            }
            if reqwest::Proxy::all(&val).is_err() {
                tracing::warn!(
                    env_var = var,
                    "proxy env var set to unparseable URL — falling through"
                );
                return None;
            }
            Some((var.to_string(), val))
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
    if let Some(result) = macos_system_proxy() {
        let env_no_proxy = no_proxy_from_env();
        return match result {
            MacosProxy::Https { url, no_proxy } => ProxyConfig {
                url: Some(url),
                no_proxy: no_proxy.or(env_no_proxy),
                source: ProxySource::MacosSystemHttps,
            },
            MacosProxy::Socks { url, no_proxy } => ProxyConfig {
                url: Some(url),
                no_proxy: no_proxy.or(env_no_proxy),
                source: ProxySource::MacosSystemSocks,
            },
            MacosProxy::PacUnsupported(pac_url) => ProxyConfig {
                url: None,
                no_proxy: None,
                source: ProxySource::MacosPacUnsupported(pac_url),
            },
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
/// unparseable as a proxy URI, the builder is returned unchanged
/// (defensive — `detect()` is supposed to validate before returning).
pub fn apply(builder: reqwest::ClientBuilder, config: &ProxyConfig) -> reqwest::ClientBuilder {
    let Some(ref url) = config.url else {
        return builder;
    };
    let Ok(proxy) = reqwest::Proxy::all(url) else {
        return builder;
    };
    // No env fallback here: `detect()` already merged env NO_PROXY into
    // `config.no_proxy`. Keeping this surface env-free is what lets the
    // OAuth client cache key (`(url, no_proxy)`) faithfully invalidate
    // when env exclusions change mid-session.
    let no_proxy = config
        .no_proxy
        .as_deref()
        .and_then(reqwest::NoProxy::from_string);
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

/// Build a proxy URL from scheme/host/port, validating the inputs.
///
/// Returns `None` if the host is empty, the port is outside the valid
/// `1..=65535` range, or the host contains an invalid character. IPv6
/// literals (containing `:`) are wrapped in brackets so the URL is
/// parseable by `reqwest::Proxy`. The host is otherwise passed through
/// unchanged — we trust SystemConfiguration's typing of the field.
//
// `dead_code` allow: only called from macOS-gated paths today, but
// the test module exercises it on every platform so Linux CI catches
// drift in this pure-string helper before anyone wires it up from
// non-macOS code. Gating the definition would break the tests.
#[allow(dead_code)]
fn format_proxy_url(scheme: &str, host: &str, port_i64: i64) -> Option<String> {
    if host.is_empty() {
        return None;
    }
    let port: u16 = port_i64.try_into().ok().filter(|&p| p > 0)?;
    if host.contains(':') && !host.starts_with('[') {
        Some(format!("{scheme}://[{host}]:{port}"))
    } else {
        Some(format!("{scheme}://{host}:{port}"))
    }
}

/// Strip `userinfo` (e.g. `user:pass@`) from a URL string for safe display.
///
/// PAC URLs occasionally embed credentials. We never want those in
/// `doctor` output or JSON. Returns the URL unchanged if no userinfo
/// component is present, or if the input doesn't have a recognizable
/// `scheme://...` shape.
//
// Same `dead_code` rationale as `format_proxy_url`: macOS-only call
// site, cross-platform tests.
#[allow(dead_code)]
fn redact_proxy_userinfo(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let after_scheme = &url[scheme_end + 3..];
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    let Some(at) = authority.find('@') else {
        return url.to_string();
    };
    let scheme = &url[..scheme_end];
    let rest = &after_scheme[at + 1..];
    format!("{scheme}://[redacted]@{rest}")
}

// ─── macOS system proxy via SystemConfiguration ──────────────────────────────

#[cfg(target_os = "macos")]
#[derive(Debug, PartialEq, Eq)]
enum MacosProxy {
    Https {
        url: String,
        no_proxy: Option<String>,
    },
    Socks {
        url: String,
        no_proxy: Option<String>,
    },
    PacUnsupported(String),
}

/// Pure priority resolver: HTTPS → SOCKS → PAC → None. Exists so the
/// priority + URL-validation logic is testable without a live
/// `SCDynamicStore`. Production callers wire CF-backed closures in
/// via `macos_system_proxy()`; tests pass closures backed by literals.
#[cfg(target_os = "macos")]
fn classify_macos_proxy<N, S, L>(get_num: N, get_str: S, read_exceptions: L) -> Option<MacosProxy>
where
    N: Fn(&str) -> Option<i64>,
    S: Fn(&str) -> Option<String>,
    L: Fn() -> Option<String>,
{
    if get_num("HTTPSEnable").unwrap_or(0) == 1 {
        if let (Some(host), Some(port)) = (get_str("HTTPSProxy"), get_num("HTTPSPort")) {
            if let Some(url) = format_proxy_url("http", &host, port) {
                return Some(MacosProxy::Https {
                    url,
                    no_proxy: read_exceptions(),
                });
            }
        }
    }

    // SOCKS fallback. `socks5h://` so DNS resolves through the proxy —
    // matches the typical Surge/Clash setup where the local resolver
    // doesn't know the upstream rules.
    if get_num("SOCKSEnable").unwrap_or(0) == 1 {
        if let (Some(host), Some(port)) = (get_str("SOCKSProxy"), get_num("SOCKSPort")) {
            if let Some(url) = format_proxy_url("socks5h", &host, port) {
                return Some(MacosProxy::Socks {
                    url,
                    no_proxy: read_exceptions(),
                });
            }
        }
    }

    // PAC: detect, do not evaluate. Evaluating FindProxyForURL needs a
    // JS engine; that's a separate decision. Surface the URL so doctor
    // can explain why traffic is going direct.
    if get_num("ProxyAutoConfigEnable").unwrap_or(0) == 1 {
        if let Some(url) = get_str("ProxyAutoConfigURLString") {
            if !url.is_empty() {
                return Some(MacosProxy::PacUnsupported(url));
            }
        }
    }

    None
}

/// Read the proxy configuration from the macOS `SystemConfiguration`
/// framework via `SCDynamicStoreCopyProxies`, dispatching priority
/// through `classify_macos_proxy()`.
#[cfg(target_os = "macos")]
fn macos_system_proxy() -> Option<MacosProxy> {
    use system_configuration::{
        core_foundation::{number::CFNumber, string::CFString},
        dynamic_store::SCDynamicStoreBuilder,
    };

    let store = SCDynamicStoreBuilder::new("claudepot").build();
    let dict = store.get_proxies()?;

    let get_num = |key: &str| -> Option<i64> {
        let cf_key = CFString::new(key);
        let val = dict.find(&cf_key)?;
        val.downcast::<CFNumber>()?.to_i64()
    };
    let get_str = |key: &str| -> Option<String> {
        let cf_key = CFString::new(key);
        let val = dict.find(&cf_key)?;
        Some(val.downcast::<CFString>()?.to_string())
    };
    let read_exceptions = || read_exceptions_list(&dict);

    classify_macos_proxy(get_num, get_str, read_exceptions)
}

/// Translate `ExceptionsList` (a `CFArray` of `CFString`) into the
/// comma-separated form `reqwest::NoProxy::from_string` expects.
///
/// `CFArray<CFString>` does not implement `ConcreteCFType`, so we
/// downcast to the untyped variant and re-wrap each element manually.
#[cfg(target_os = "macos")]
fn read_exceptions_list(
    dict: &system_configuration::core_foundation::dictionary::CFDictionary<
        system_configuration::core_foundation::string::CFString,
        system_configuration::core_foundation::base::CFType,
    >,
) -> Option<String> {
    use std::ffi::c_void;
    use system_configuration::core_foundation::{array::CFArray, base::TCFType, string::CFString};

    let exc_key = CFString::new("ExceptionsList");
    let val = dict.find(&exc_key)?;
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
        {
            assert_eq!(
                ProxySource::MacosSystemHttps.to_string(),
                "SystemConfiguration (HTTPS)"
            );
            assert_eq!(
                ProxySource::MacosSystemSocks.to_string(),
                "SystemConfiguration (SOCKS)"
            );
            assert_eq!(
                ProxySource::MacosPacUnsupported("http://wpad/wpad.dat".into()).to_string(),
                "PAC configured but unsupported: http://wpad/wpad.dat (going direct)"
            );
            // PAC URL with embedded creds — display must redact userinfo.
            assert_eq!(
                ProxySource::MacosPacUnsupported(
                    "http://alice:secret@wpad.corp.local/proxy.pac".into()
                )
                .to_string(),
                "PAC configured but unsupported: http://[redacted]@wpad.corp.local/proxy.pac (going direct)"
            );
        }
    }

    #[test]
    fn test_is_warning_only_for_pac() {
        assert!(!ProxySource::None.is_warning());
        assert!(!ProxySource::EnvVar("https_proxy".into()).is_warning());
        #[cfg(target_os = "macos")]
        {
            assert!(!ProxySource::MacosSystemHttps.is_warning());
            assert!(!ProxySource::MacosSystemSocks.is_warning());
            assert!(ProxySource::MacosPacUnsupported("http://x/p.pac".into()).is_warning());
        }
    }

    #[test]
    fn test_apply_no_url_passthrough() {
        let cfg = ProxyConfig {
            url: None,
            no_proxy: None,
            source: ProxySource::None,
        };
        let client = apply(reqwest::Client::builder(), &cfg).build();
        assert!(client.is_ok());
    }

    #[test]
    fn test_apply_with_valid_url() {
        let cfg = ProxyConfig {
            url: Some("http://127.0.0.1:6152".into()),
            no_proxy: Some("localhost,*.local".into()),
            source: ProxySource::EnvVar("HTTPS_PROXY".into()),
        };
        let client = apply(reqwest::Client::builder(), &cfg).build();
        assert!(client.is_ok());
    }

    #[test]
    fn test_socks_feature_is_enabled() {
        // Direct probe: if the `socks` reqwest feature is missing, this
        // returns Err and the test fails. apply()-based assertions can
        // hide that because apply() degrades silently to direct.
        assert!(reqwest::Proxy::all("socks5h://127.0.0.1:7891").is_ok());
    }

    #[test]
    fn test_apply_with_socks_url() {
        let cfg = ProxyConfig {
            url: Some("socks5h://127.0.0.1:7891".into()),
            no_proxy: None,
            source: ProxySource::None,
        };
        let client = apply(reqwest::Client::builder(), &cfg).build();
        assert!(client.is_ok());
    }

    #[test]
    fn test_apply_with_unparseable_url_does_not_panic() {
        let cfg = ProxyConfig {
            url: Some("not a url".into()),
            no_proxy: None,
            source: ProxySource::None,
        };
        let client = apply(reqwest::Client::builder(), &cfg).build();
        assert!(client.is_ok());
    }

    #[test]
    fn test_format_proxy_url_ipv4() {
        assert_eq!(
            format_proxy_url("http", "127.0.0.1", 6152),
            Some("http://127.0.0.1:6152".into())
        );
    }

    #[test]
    fn test_format_proxy_url_ipv6_brackets() {
        assert_eq!(
            format_proxy_url("http", "::1", 6152),
            Some("http://[::1]:6152".into())
        );
        assert_eq!(
            format_proxy_url("http", "fe80::1", 8080),
            Some("http://[fe80::1]:8080".into())
        );
    }

    #[test]
    fn test_format_proxy_url_already_bracketed() {
        // If a host already starts with '[' (uncommon but possible),
        // we don't re-bracket.
        assert_eq!(
            format_proxy_url("http", "[::1]", 80),
            Some("http://[::1]:80".into())
        );
    }

    #[test]
    fn test_format_proxy_url_rejects_bad_port() {
        assert_eq!(format_proxy_url("http", "127.0.0.1", 0), None);
        assert_eq!(format_proxy_url("http", "127.0.0.1", -1), None);
        assert_eq!(format_proxy_url("http", "127.0.0.1", 65536), None);
        assert_eq!(format_proxy_url("http", "127.0.0.1", i64::MAX), None);
    }

    #[test]
    fn test_format_proxy_url_rejects_empty_host() {
        assert_eq!(format_proxy_url("http", "", 8080), None);
    }

    #[test]
    fn test_redact_proxy_userinfo_passthrough() {
        assert_eq!(
            redact_proxy_userinfo("http://wpad/wpad.dat"),
            "http://wpad/wpad.dat"
        );
        assert_eq!(redact_proxy_userinfo("not a url"), "not a url");
    }

    #[test]
    fn test_redact_proxy_userinfo_strips_creds() {
        assert_eq!(
            redact_proxy_userinfo("http://alice:secret@host/path"),
            "http://[redacted]@host/path"
        );
        assert_eq!(
            redact_proxy_userinfo("https://user@example.com/p.pac"),
            "https://[redacted]@example.com/p.pac"
        );
    }

    #[test]
    fn test_redact_proxy_userinfo_ignores_at_in_path() {
        // '@' in path/query must NOT be treated as userinfo delimiter.
        assert_eq!(
            redact_proxy_userinfo("http://example.com/path@query"),
            "http://example.com/path@query"
        );
        assert_eq!(
            redact_proxy_userinfo("http://example.com/?q=foo@bar"),
            "http://example.com/?q=foo@bar"
        );
    }

    // ─── classify_macos_proxy: closure-driven priority/validation tests ────
    //
    // These are macOS-only because MacosProxy is macOS-gated. The logic
    // they cover (HTTPS > SOCKS > PAC, port range, IPv6 host) is
    // platform-independent; we just can't reference the type elsewhere.

    #[cfg(target_os = "macos")]
    #[test]
    fn test_classify_https_wins_when_enabled() {
        let result = classify_macos_proxy(
            |k| match k {
                "HTTPSEnable" => Some(1),
                "HTTPSPort" => Some(6152),
                "SOCKSEnable" => Some(1), // present but should be ignored
                "SOCKSPort" => Some(7891),
                _ => None,
            },
            |k| match k {
                "HTTPSProxy" => Some("127.0.0.1".into()),
                "SOCKSProxy" => Some("127.0.0.1".into()),
                _ => None,
            },
            || Some("localhost,*.local".into()),
        );
        assert_eq!(
            result,
            Some(MacosProxy::Https {
                url: "http://127.0.0.1:6152".into(),
                no_proxy: Some("localhost,*.local".into()),
            })
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_classify_socks_when_no_https() {
        let result = classify_macos_proxy(
            |k| match k {
                "SOCKSEnable" => Some(1),
                "SOCKSPort" => Some(7891),
                _ => None,
            },
            |k| match k {
                "SOCKSProxy" => Some("127.0.0.1".into()),
                _ => None,
            },
            || None,
        );
        assert_eq!(
            result,
            Some(MacosProxy::Socks {
                url: "socks5h://127.0.0.1:7891".into(),
                no_proxy: None,
            })
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_classify_pac_when_no_https_or_socks() {
        let result = classify_macos_proxy(
            |k| match k {
                "ProxyAutoConfigEnable" => Some(1),
                _ => None,
            },
            |k| match k {
                "ProxyAutoConfigURLString" => Some("http://wpad/wpad.dat".into()),
                _ => None,
            },
            || None,
        );
        assert_eq!(
            result,
            Some(MacosProxy::PacUnsupported("http://wpad/wpad.dat".into()))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_classify_none_when_nothing_enabled() {
        let result = classify_macos_proxy(|_| None, |_| None, || None);
        assert_eq!(result, None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_classify_https_with_invalid_port_falls_through_to_socks() {
        // HTTPSEnable=1 but port is out-of-range. We must not synthesize
        // a malformed URL — we should fall through to SOCKS.
        let result = classify_macos_proxy(
            |k| match k {
                "HTTPSEnable" => Some(1),
                "HTTPSPort" => Some(99999), // invalid
                "SOCKSEnable" => Some(1),
                "SOCKSPort" => Some(7891),
                _ => None,
            },
            |k| match k {
                "HTTPSProxy" => Some("127.0.0.1".into()),
                "SOCKSProxy" => Some("127.0.0.1".into()),
                _ => None,
            },
            || None,
        );
        assert!(matches!(result, Some(MacosProxy::Socks { .. })));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_classify_https_with_ipv6_host() {
        let result = classify_macos_proxy(
            |k| match k {
                "HTTPSEnable" => Some(1),
                "HTTPSPort" => Some(6152),
                _ => None,
            },
            |k| match k {
                "HTTPSProxy" => Some("::1".into()),
                _ => None,
            },
            || None,
        );
        assert_eq!(
            result,
            Some(MacosProxy::Https {
                url: "http://[::1]:6152".into(),
                no_proxy: None,
            })
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_classify_pac_skips_empty_url() {
        let result = classify_macos_proxy(
            |k| match k {
                "ProxyAutoConfigEnable" => Some(1),
                _ => None,
            },
            |k| match k {
                "ProxyAutoConfigURLString" => Some(String::new()),
                _ => None,
            },
            || None,
        );
        assert_eq!(result, None);
    }
}
