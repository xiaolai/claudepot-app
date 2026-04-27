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

    // Strip path/query/fragment to get the authority component.
    let authority_end = after_scheme
        .find(|c: char| c == '/' || c == '?' || c == '#')
        .unwrap_or(after_scheme.len());
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
        let err =
            validate_base_url("https://user:pass@example.com").unwrap_err();
        assert!(matches!(err, BaseUrlError::Malformed(_)));
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
        let err =
            validate_base_url("https://example.com/with space").unwrap_err();
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
}
