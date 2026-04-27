//! Wrapper-name slug derivation. Per design §4:
//!
//! 1. Strip recognized provider prefixes (moonshotai/, anthropic/,
//!    us./eu./ap./ regions, anthropic.).
//! 2. Lowercase.
//! 3. Replace `[: / . _ <space>]` with `-`.
//! 4. Collapse repeated `-`.
//! 5. Strip leading `claude-` to avoid `claude-claude-…`.
//! 6. Cap at 24 chars at a `-` boundary if possible.
//!
//! Then prefix with `claude-` to get the wrapper binary name.

use thiserror::Error;

const MAX_SLUG_LEN: usize = 24;
const PREFIXES_TO_STRIP: &[&str] = &[
    "moonshotai/",
    "anthropic/",
    "anthropic.",
    "us.",
    "eu.",
    "ap.",
];

#[derive(Debug, Error)]
pub enum WrapperNameError {
    #[error("wrapper name cannot be empty")]
    Empty,
    #[error("wrapper name '{0}' contains characters that aren't shell-safe (allowed: alnum, dash, underscore)")]
    InvalidChars(String),
    #[error("wrapper name '{0}' is reserved")]
    Reserved(String),
}

/// Reserved binary names Claudepot refuses to write a wrapper for.
const RESERVED: &[&str] = &["claude", "sh", "bash", "zsh", "fish", "env"];

/// Validate that a wrapper name is shell-safe and non-reserved.
/// Used both for auto-derived slugs and for user overrides.
pub fn sanitize_wrapper_name(name: &str) -> Result<String, WrapperNameError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(WrapperNameError::Empty);
    }
    if RESERVED.contains(&trimmed) {
        return Err(WrapperNameError::Reserved(trimmed.to_string()));
    }
    // Allow alnum + dash + underscore. No leading dash. No path
    // separators. Keep the rule narrow so we don't fight the
    // shell — `man 7 path_resolution` covers the rest.
    if trimmed.starts_with('-')
        || trimmed
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    {
        return Err(WrapperNameError::InvalidChars(trimmed.to_string()));
    }
    Ok(trimmed.to_string())
}

/// Derive the default wrapper name from a model field. Returns the
/// full binary name including the `claude-` prefix.
pub fn derive_wrapper_slug(model: &str) -> String {
    let mut s = model.trim().to_string();

    // 1. Strip recognized provider prefixes (case-sensitive on the
    //    forms Anthropic publishes; user-typed weirdness falls
    //    through and gets sanitized below).
    for prefix in PREFIXES_TO_STRIP {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_string();
            break;
        }
    }

    // 2. Lowercase.
    s = s.to_ascii_lowercase();

    // 3. Replace `[: / . _ space]` with `-`.
    s = s
        .chars()
        .map(|c| match c {
            ':' | '/' | '.' | '_' | ' ' => '-',
            other => other,
        })
        .collect::<String>();

    // Drop any character we don't allow at all (anything that's
    // not alnum or `-`). Belt-and-braces — the regex providers use
    // doesn't include exotic punctuation, but if a user's model id
    // does, drop it rather than carry through.
    s = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>();

    // 4. Collapse repeated `-`.
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    s = s.trim_matches('-').to_string();

    // 5. Strip leading `claude-` to avoid double prefix.
    while let Some(rest) = s.strip_prefix("claude-") {
        s = rest.to_string();
    }
    if s == "claude" {
        s = String::new();
    }

    if s.is_empty() {
        // Pathological input — fall back to a generic placeholder
        // the user is expected to override.
        return String::from("claude-route");
    }

    // 6. Cap at MAX_SLUG_LEN chars at a `-` boundary if possible.
    if s.len() > MAX_SLUG_LEN {
        // Walk backward from MAX_SLUG_LEN to find a dash; if none
        // before position 8 (avoid hyper-truncation), hard-cut.
        let mut cut = MAX_SLUG_LEN;
        if let Some(last_dash) = s[..MAX_SLUG_LEN].rfind('-') {
            if last_dash >= 8 {
                cut = last_dash;
            }
        }
        s.truncate(cut);
        s = s.trim_end_matches('-').to_string();
    }

    format!("claude-{s}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_style() {
        assert_eq!(derive_wrapper_slug("llama3.2:3b"), "claude-llama3-2-3b");
    }

    #[test]
    fn moonshot_prefix_stripped() {
        assert_eq!(
            derive_wrapper_slug("moonshotai/kimi-k2"),
            "claude-kimi-k2"
        );
    }

    #[test]
    fn deepseek_dot_replaced() {
        assert_eq!(
            derive_wrapper_slug("deepseek-v3.2"),
            "claude-deepseek-v3-2"
        );
    }

    #[test]
    fn anthropic_bedrock_long_id_truncated() {
        let slug =
            derive_wrapper_slug("us.anthropic.claude-sonnet-4-20250514-v1:0");
        assert!(slug.starts_with("claude-"));
        assert!(slug.len() - "claude-".len() <= MAX_SLUG_LEN);
        assert!(!slug.ends_with('-'));
        // Should not produce `claude-claude-` because the leading
        // `claude-` after the `anthropic.` strip gets removed.
        assert!(!slug.starts_with("claude-claude"));
    }

    #[test]
    fn gpt_dot_replaced() {
        assert_eq!(derive_wrapper_slug("gpt-5.4"), "claude-gpt-5-4");
    }

    #[test]
    fn empty_falls_back() {
        assert_eq!(derive_wrapper_slug(""), "claude-route");
        assert_eq!(derive_wrapper_slug("   "), "claude-route");
    }

    #[test]
    fn pure_claude_falls_back() {
        // After stripping `claude-`, nothing left.
        assert_eq!(derive_wrapper_slug("claude"), "claude-route");
        assert_eq!(derive_wrapper_slug("claude-"), "claude-route");
    }

    #[test]
    fn already_dashed_passes_through() {
        assert_eq!(derive_wrapper_slug("kimi-k2"), "claude-kimi-k2");
    }

    #[test]
    fn weird_chars_dropped() {
        // `@` and `!` are dropped, not replaced with `-`.
        assert_eq!(
            derive_wrapper_slug("foo@bar!baz"),
            "claude-foobarbaz"
        );
    }

    #[test]
    fn uppercase_lowercased() {
        assert_eq!(derive_wrapper_slug("LLaMa3"), "claude-llama3");
    }

    #[test]
    fn collapses_repeated_dashes() {
        assert_eq!(
            derive_wrapper_slug("a--b---c"),
            "claude-a-b-c"
        );
    }

    #[test]
    fn sanitize_wrapper_name_accepts_typical() {
        assert!(sanitize_wrapper_name("claude-ollama").is_ok());
        assert!(sanitize_wrapper_name("kimi").is_ok());
        assert!(sanitize_wrapper_name("claude-bedrock-prod").is_ok());
    }

    #[test]
    fn sanitize_wrapper_name_rejects_empty() {
        assert!(matches!(
            sanitize_wrapper_name("").unwrap_err(),
            WrapperNameError::Empty
        ));
        assert!(matches!(
            sanitize_wrapper_name("   ").unwrap_err(),
            WrapperNameError::Empty
        ));
    }

    #[test]
    fn sanitize_wrapper_name_rejects_reserved() {
        for r in ["claude", "sh", "bash", "zsh"] {
            assert!(matches!(
                sanitize_wrapper_name(r).unwrap_err(),
                WrapperNameError::Reserved(_)
            ));
        }
    }

    #[test]
    fn sanitize_wrapper_name_rejects_path_chars() {
        assert!(sanitize_wrapper_name("claude/ollama").is_err());
        assert!(sanitize_wrapper_name("claude ollama").is_err());
        assert!(sanitize_wrapper_name(".claude").is_err());
        assert!(sanitize_wrapper_name("-claude").is_err());
    }
}
