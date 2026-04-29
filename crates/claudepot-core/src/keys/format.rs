//! Token classification and truncated-preview rendering.
//!
//! Two supported prefixes:
//! * `sk-ant-api03-…` — Anthropic API key (console-issued, billing)
//! * `sk-ant-oat01-…` — OAuth access token (issued by `claude
//!   setup-token`, 1-year validity, billed against the account that
//!   issued it)
//!
//! Preview format matches the convention stated in
//! `.claude/rules/rust-conventions.md`:
//!     `sk-ant-oat01-Abc…xyz`
//! First 3 characters after the prefix, an ellipsis, last 3 of the
//! opaque body. Never round-trip the full value through logs or DTOs.

pub const API_KEY_PREFIX: &str = "sk-ant-api03-";
pub const OAUTH_TOKEN_PREFIX: &str = "sk-ant-oat01-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPrefix {
    ApiKey,
    OauthToken,
}

pub fn classify_token(token: &str) -> Option<KeyPrefix> {
    // A bare prefix (e.g. just `sk-ant-api03-`) is not a real token;
    // require a non-empty opaque body so the "paste the full value"
    // contract the add flow depends on actually holds. The body must
    // also be shell-safe — `key oauth copy-shell` and similar paths
    // compose this value into shell text, so quotes, whitespace, and
    // control chars must be rejected at classification time, not
    // patched up later at copy time.
    if let Some(body) = token.strip_prefix(API_KEY_PREFIX) {
        if !body.is_empty() && is_token_body_safe(body) {
            return Some(KeyPrefix::ApiKey);
        }
    }
    if let Some(body) = token.strip_prefix(OAUTH_TOKEN_PREFIX) {
        if !body.is_empty() && is_token_body_safe(body) {
            return Some(KeyPrefix::OauthToken);
        }
    }
    None
}

/// Anthropic-issued tokens are base64url-shaped: alnum plus `-` and `_`.
/// Anything else (quotes, whitespace, control chars, shell metachars)
/// is either a malformed paste or an attempted injection — refuse.
fn is_token_body_safe(body: &str) -> bool {
    body.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Truncate `sk-ant-oat01-ABCDEF...XYZQ` to `sk-ant-oat01-ABC…XYZ`.
/// Returns the input unchanged when it's too short to safely redact
/// (under 20 characters) — that case only happens on malformed input
/// and we prefer to surface the string rather than pretend to redact.
pub fn token_preview(token: &str) -> String {
    let prefix_len = match classify_token(token) {
        Some(KeyPrefix::ApiKey) => API_KEY_PREFIX.len(),
        Some(KeyPrefix::OauthToken) => OAUTH_TOKEN_PREFIX.len(),
        None => return safe_generic_preview(token),
    };
    if token.len() < prefix_len + 10 {
        // Token body too short to redact with head + tail without
        // overlapping — fall back to full opaque preview.
        return format!("{}…", &token[..prefix_len.min(token.len())]);
    }
    let prefix = &token[..prefix_len];
    let body = &token[prefix_len..];
    // Use char_indices so a multi-byte tail doesn't panic — tokens are
    // ASCII today, but the defensive accounting is cheap.
    let head: String = body.chars().take(3).collect();
    let tail: String = body
        .chars()
        .rev()
        .take(3)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{prefix}{head}…{tail}")
}

fn safe_generic_preview(token: &str) -> String {
    if token.len() <= 8 {
        return "…".to_string();
    }
    let head: String = token.chars().take(4).collect();
    let tail: String = token
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_api_key() {
        assert_eq!(
            classify_token("sk-ant-api03-ABCDEFG_hijklmnop-xyz"),
            Some(KeyPrefix::ApiKey)
        );
    }

    #[test]
    fn classify_oauth_token() {
        assert_eq!(
            classify_token("sk-ant-oat01-aaabbbcccdddeee-fff"),
            Some(KeyPrefix::OauthToken)
        );
    }

    #[test]
    fn classify_rejects_unknown() {
        assert!(classify_token("sk-ant-something-else").is_none());
        assert!(classify_token("").is_none());
        assert!(classify_token("hello world").is_none());
    }

    #[test]
    fn classify_rejects_bare_prefix() {
        assert!(classify_token("sk-ant-api03-").is_none());
        assert!(classify_token("sk-ant-oat01-").is_none());
    }

    #[test]
    fn preview_api_key_redacts_middle() {
        let preview = token_preview("sk-ant-api03-AbCdEfGhIjKlMn_oPqRsTuVwXyZ");
        assert_eq!(preview, "sk-ant-api03-AbC…XyZ");
    }

    #[test]
    fn preview_oauth_token_redacts_middle() {
        let preview = token_preview("sk-ant-oat01-Hello1234567890abcdefXyz");
        assert_eq!(preview, "sk-ant-oat01-Hel…Xyz");
    }

    #[test]
    fn preview_short_token_is_safe() {
        // Below threshold — we still redact rather than echoing the value.
        let preview = token_preview("sk-ant-api03-abc");
        assert!(!preview.contains("abc"));
    }

    #[test]
    fn preview_unknown_prefix_uses_generic_redaction() {
        let preview = token_preview("sk-not-a-real-prefix-abcdefg");
        assert!(preview.contains('…'));
        assert!(!preview.contains("abcdefg"));
    }
}
