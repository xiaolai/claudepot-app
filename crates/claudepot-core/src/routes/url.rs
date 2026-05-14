//! Lightweight base-URL validator. Replaces the `starts_with("http")`
//! check the Tauri command was using and gives every provider the
//! same gate.
//!
//! We don't pull in the full `url` crate just for this — a route
//! base URL must be `(http|https)://<host>[:<port>][/<path>]` with
//! a non-empty host and no embedded whitespace. Anything more
//! exotic (auth in the URL, fragments, queries) is not what
//! Anthropic, AWS, GCP, or Azure document, so we reject it.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BaseUrlError {
    #[error("URL is empty")]
    Empty,
    #[error("URL must use http:// or https:// (got: {0})")]
    BadScheme(String),
    #[error("URL has no host")]
    NoHost,
    #[error("URL contains whitespace or control characters")]
    InvalidChars,
    #[error("URL is malformed: {0}")]
    Malformed(String),
}

/// Validate a base URL. Returns the trimmed string on success.
pub fn validate_base_url(input: &str) -> Result<String, BaseUrlError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(BaseUrlError::Empty);
    }
    if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(BaseUrlError::InvalidChars);
    }

    let after_scheme = if let Some(rest) = s.strip_prefix("https://") {
        rest
    } else if let Some(rest) = s.strip_prefix("http://") {
        rest
    } else {
        return Err(BaseUrlError::BadScheme(s.to_string()));
    };

    // Reject query strings and fragments. No provider base URL
    // (Anthropic, AWS, GCP, Azure, or a self-hosted gateway) carries
    // them, and leaving one in produces a malformed request URL once
    // the SDK appends `/v1/messages` (e.g. `…/v1?x=y/v1/messages`).
    // The module header promises these are rejected — enforce it.
    if after_scheme.contains(['?', '#']) {
        return Err(BaseUrlError::Malformed(
            "query strings and fragments are not allowed in a base URL".into(),
        ));
    }

    // Strip the path to get the authority component. Query/fragment
    // were already rejected above, so the path starts at the first
    // `/`.
    let authority_end = after_scheme.find('/').unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];

    if authority.is_empty() {
        return Err(BaseUrlError::NoHost);
    }

    // Reject userinfo (we don't allow `user:pass@host`).
    if authority.contains('@') {
        return Err(BaseUrlError::Malformed(
            "userinfo (`user:pass@host`) is not allowed in base URLs".into(),
        ));
    }

    // Split off optional port. Note IPv6 literal handling: `[::1]:1234`.
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        let end = stripped
            .find(']')
            .ok_or_else(|| BaseUrlError::Malformed("unterminated IPv6 literal".into()))?;
        let h = &stripped[..end];
        let after = &stripped[end + 1..];
        if let Some(port_part) = after.strip_prefix(':') {
            if port_part.parse::<u16>().is_err() {
                return Err(BaseUrlError::Malformed(format!(
                    "invalid port: {port_part}"
                )));
            }
        } else if !after.is_empty() {
            return Err(BaseUrlError::Malformed(
                "trailing characters after IPv6 literal".into(),
            ));
        }
        h
    } else if let Some((h, port)) = authority.rsplit_once(':') {
        if port.parse::<u16>().is_err() {
            return Err(BaseUrlError::Malformed(format!("invalid port: {port}")));
        }
        h
    } else {
        authority
    };

    if host.is_empty() {
        return Err(BaseUrlError::NoHost);
    }

    Ok(s.to_string())
}

/// Normalize a **gateway** base URL: validate it, then strip a
/// trailing `/v1` (or `/v1/`) path.
///
/// Claude Code's Anthropic SDK appends `/v1/messages` to
/// `ANTHROPIC_BASE_URL` itself. But every Ollama / OpenAI-compatible
/// doc tells users to point at the `…/v1` URL, so users naturally
/// paste `http://host:11434/v1` — which then resolves to
/// `…/v1/v1/messages`, a 404 that Claude Code surfaces as a
/// misleading "model may not exist". Stripping the trailing `/v1`
/// on save makes the pasted value work as the user expects.
///
/// Scope is deliberately narrow: only the bare `/v1` path segment is
/// stripped, and only for gateway routes. Bedrock / Vertex / Foundry
/// base URLs carry provider-specific path semantics and must not be
/// touched — they keep using [`validate_base_url`] directly.
pub fn normalize_gateway_base_url(input: &str) -> Result<String, BaseUrlError> {
    let validated = validate_base_url(input)?;

    // Only strip `/v1` when it is the URL *path* — never the
    // scheme/authority, so a host literally named `v1` survives.
    let scheme_len = if validated.starts_with("https://") {
        "https://".len()
    } else {
        "http://".len()
    };
    let Some(path_start) = validated[scheme_len..]
        .find(['/', '?', '#'])
        .map(|i| scheme_len + i)
    else {
        return Ok(validated);
    };

    let (base, path) = validated.split_at(path_start);
    if path == "/v1" || path == "/v1/" {
        Ok(base.to_string())
    } else {
        Ok(validated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_typical_urls() {
        for ok in [
            "http://127.0.0.1:11434",
            "https://api.example.com",
            "https://api.example.com/v1",
            "http://localhost",
            "http://localhost:3000/anthropic",
            "https://api.openrouter.ai/api/v1",
            "https://[::1]:11434",
            "https://[2001:db8::1]:8080/api",
        ] {
            assert!(
                validate_base_url(ok).is_ok(),
                "should accept {ok:?}, got {:?}",
                validate_base_url(ok),
            );
        }
    }

    #[test]
    fn rejects_empty() {
        assert!(matches!(
            validate_base_url("").unwrap_err(),
            BaseUrlError::Empty
        ));
        assert!(matches!(
            validate_base_url("   ").unwrap_err(),
            BaseUrlError::Empty
        ));
    }

    #[test]
    fn rejects_bad_scheme() {
        for bad in [
            "ftp://example.com",
            "example.com",
            "//example.com",
            "javascript:alert(1)",
            "file:///etc/passwd",
        ] {
            let err = validate_base_url(bad).unwrap_err();
            assert!(
                matches!(err, BaseUrlError::BadScheme(_)),
                "should reject {bad:?}, got {err:?}",
            );
        }
    }

    #[test]
    fn rejects_userinfo() {
        let err = validate_base_url("https://user:pass@example.com").unwrap_err();
        assert!(matches!(err, BaseUrlError::Malformed(_)));
    }

    #[test]
    fn rejects_query_and_fragment() {
        for bad in [
            "https://api.example.com/v1?token=abc",
            "http://host:11434/v1#frag",
            "https://host?x=y",
        ] {
            let err = validate_base_url(bad).unwrap_err();
            assert!(
                matches!(err, BaseUrlError::Malformed(_)),
                "should reject {bad:?}, got {err:?}",
            );
        }
    }

    #[test]
    fn rejects_no_host() {
        for bad in ["https://", "http:///path", "https:///"] {
            let err = validate_base_url(bad).unwrap_err();
            assert!(
                matches!(err, BaseUrlError::NoHost),
                "should reject {bad:?} with NoHost, got {err:?}",
            );
        }
    }

    #[test]
    fn rejects_inner_whitespace() {
        // Inner whitespace in URLs is invalid.
        let err = validate_base_url("https://example.com/with space").unwrap_err();
        assert!(matches!(err, BaseUrlError::InvalidChars));
        // Inner control char in middle of host.
        let err = validate_base_url("https://exam\nple.com").unwrap_err();
        assert!(matches!(err, BaseUrlError::InvalidChars));
    }

    #[test]
    fn trims_outer_whitespace_courteously() {
        // Leading/trailing whitespace is a paste accident; accept.
        assert!(validate_base_url("  https://example.com  ").is_ok());
        assert!(validate_base_url("https://example.com\n").is_ok());
    }

    #[test]
    fn rejects_bad_port() {
        for bad in [
            "http://example.com:abc",
            "http://example.com:99999",
            "http://[::1]:abc",
        ] {
            let err = validate_base_url(bad).unwrap_err();
            assert!(
                matches!(err, BaseUrlError::Malformed(_)),
                "should reject {bad:?}, got {err:?}",
            );
        }
    }

    #[test]
    fn normalize_gateway_strips_trailing_v1() {
        assert_eq!(
            normalize_gateway_base_url("http://100.100.1.6:11434/v1").unwrap(),
            "http://100.100.1.6:11434",
        );
        // Trailing slash variant.
        assert_eq!(
            normalize_gateway_base_url("https://api.example.com/v1/").unwrap(),
            "https://api.example.com",
        );
        // Paste accident with surrounding whitespace still normalizes.
        assert_eq!(
            normalize_gateway_base_url("  http://host:8080/v1  ").unwrap(),
            "http://host:8080",
        );
    }

    #[test]
    fn normalize_gateway_leaves_other_urls_untouched() {
        for unchanged in [
            "http://100.100.1.6:11434",
            "https://api.example.com",
            // Only the *bare* `/v1` segment is stripped.
            "http://host:11434/v1/messages",
            "https://api.openrouter.ai/api/v1",
            "http://localhost:3000/anthropic",
        ] {
            assert_eq!(
                normalize_gateway_base_url(unchanged).unwrap(),
                unchanged.trim(),
                "should pass {unchanged:?} through unchanged",
            );
        }
    }

    #[test]
    fn normalize_gateway_never_touches_authority() {
        // A host literally named `v1` must survive — the `/v1` strip
        // applies to the path only, never the scheme/authority.
        assert_eq!(
            normalize_gateway_base_url("https://v1").unwrap(),
            "https://v1",
        );
        assert_eq!(
            normalize_gateway_base_url("http://v1:11434").unwrap(),
            "http://v1:11434",
        );
    }

    #[test]
    fn normalize_gateway_propagates_validation_errors() {
        assert!(matches!(
            normalize_gateway_base_url("").unwrap_err(),
            BaseUrlError::Empty
        ));
        assert!(matches!(
            normalize_gateway_base_url("ftp://example.com/v1").unwrap_err(),
            BaseUrlError::BadScheme(_)
        ));
    }
}
