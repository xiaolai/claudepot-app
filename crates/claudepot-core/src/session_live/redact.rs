//! M1 redaction floor — exact port of
//! `src/sections/sessions/viewers/redact.ts`.
//!
//! Matches `sk-ant-<tokenchars>` where tokenchars are alphanumerics,
//! `-`, or `_`. Short tokens (≤12 chars including prefix) are masked
//! completely; longer tokens keep their last four characters so two
//! different leaks remain distinguishable.
//!
//! This is the canonical trust boundary. Every field that crosses
//! out of `claudepot-core::session_live` into the Tauri DTO layer
//! must pass through `redact` (or a later family that extends it).
//! Tauri-side and TS-side redaction remain as belt-and-braces, not
//! as the authority.
//!
//! Extended redaction families (Bearer/JWT, Authorization headers,
//! password/api_key params, cookies) are deliberately deferred to
//! M2 and will each ship with fixtures covering both positive and
//! negative cases. See plan §7.

use once_cell::sync::Lazy;
use regex::Regex;

/// Anthropic key family. `sk-ant-` plus one or more of
/// `[A-Za-z0-9_-]`. Matches OAuth variants (`sk-ant-oat01-...`)
/// because the prefix family is shared.
static SK_ANT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"sk-ant-[A-Za-z0-9_-]+").expect("static regex"));

/// Length threshold below which a matched token is masked entirely
/// rather than keeping a suffix. Matches `redact.ts`.
const SHORT_TOKEN_THRESHOLD: usize = 12;

/// Redact any `sk-ant-*` tokens in `text`. Safe to call on strings
/// that contain no match (fast path: substring check first, same
/// optimization as the TS side).
pub fn redact_secrets(text: &str) -> String {
    if !text.contains("sk-ant-") {
        return text.to_string();
    }
    SK_ANT_RE
        .replace_all(text, |caps: &regex::Captures<'_>| {
            let tok = &caps[0];
            if tok.len() <= SHORT_TOKEN_THRESHOLD {
                "sk-ant-***".to_string()
            } else {
                let tail = &tok[tok.len() - 4..];
                format!("sk-ant-***{tail}")
            }
        })
        .into_owned()
}

/// Convenience wrapper for `Option<&str>` call sites — passes through
/// `None` and empty strings unchanged, redacting only populated input.
pub fn redact_secrets_opt(text: Option<&str>) -> String {
    match text {
        None => String::new(),
        Some(s) => redact_secrets(s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirror of `redact.ts`: strings without the sentinel prefix
    /// pass through untouched. The early return is also a perf path,
    /// so assert the behavior, not just the result.
    #[test]
    fn no_match_returns_input_verbatim() {
        let input = "no secrets here";
        let output = redact_secrets(input);
        assert_eq!(output, input);
    }

    #[test]
    fn empty_and_whitespace_are_no_ops() {
        assert_eq!(redact_secrets(""), "");
        assert_eq!(redact_secrets("   "), "   ");
    }

    #[test]
    fn short_token_is_fully_masked() {
        // `sk-ant-a` = 8 chars, well under the 12 threshold.
        let out = redact_secrets("leading sk-ant-a trailing");
        assert_eq!(out, "leading sk-ant-*** trailing");
    }

    #[test]
    fn exactly_twelve_char_token_is_fully_masked() {
        // `sk-ant-` (7) + `abcde` (5) = 12 chars — matches the `<=` branch.
        let out = redact_secrets("x sk-ant-abcde y");
        assert_eq!(out, "x sk-ant-*** y");
    }

    #[test]
    fn long_token_keeps_last_four() {
        // `sk-ant-Abc123DEF456_ghiJKLxyz` = 29 chars, keeps `Jxyz`.
        let out = redact_secrets("key=sk-ant-Abc123DEF456_ghiJKLxyz rest");
        assert_eq!(out, "key=sk-ant-***Lxyz rest");
    }

    #[test]
    fn oauth_variant_is_redacted() {
        // OAuth access tokens share the `sk-ant-` prefix via `oat01-`
        // — they MUST be caught by the same pattern.
        let tok = "sk-ant-oat01-Abc_123-DEFghiJKL-xyz";
        let out = redact_secrets(&format!("token={tok}"));
        assert!(!out.contains(tok));
        assert!(out.starts_with("token=sk-ant-***"));
        assert!(out.ends_with("-xyz"));
    }

    #[test]
    fn multiple_tokens_all_redacted() {
        let input =
            "a sk-ant-shortt b sk-ant-longerThanTwelveChars c sk-ant-x d";
        let out = redact_secrets(input);
        assert!(!out.contains("shortt"));
        assert!(!out.contains("longerThanTwelveChars"));
        // The short `sk-ant-x` (8 chars) collapses to the fully-masked form.
        assert!(out.contains("sk-ant-***"));
        assert!(!out.contains("sk-ant-x "));
    }

    /// Regex matches are greedy on `[A-Za-z0-9_-]` — surrounding
    /// punctuation terminates the match so we don't swallow JSON
    /// delimiters like `"` or `,`.
    #[test]
    fn stops_at_non_token_character() {
        let input = r#"{"key":"sk-ant-Abc123DEF456_xyz","other":"v"}"#;
        let out = redact_secrets(input);
        assert!(out.contains(r#""other":"v""#));
        assert!(!out.contains("Abc123"));
    }

    /// Redaction is a projection: applying it a second time must
    /// not change the result. If it did, the masked form would itself
    /// contain something that looked like a key, which is a sign of
    /// an incorrect replacement shape.
    #[test]
    fn redaction_is_idempotent() {
        let cases = [
            "plain text",
            "sk-ant-a",
            "sk-ant-abcde",
            "sk-ant-Abc123DEF456_ghiJKLxyz",
            "sk-ant-oat01-Abc_123-DEFghiJKL-xyz",
            "multi sk-ant-short sk-ant-longerkeyABCD xyz",
            r#"{"auth":"sk-ant-abcdefghij"}"#,
        ];
        for c in &cases {
            let once = redact_secrets(c);
            let twice = redact_secrets(&once);
            assert_eq!(once, twice, "not idempotent on: {c}");
        }
    }

    /// Property-ish test: for any replacement output, the substring
    /// `sk-ant-` only appears in the sanitized `sk-ant-***` form.
    /// Anything longer means a real token leaked through.
    #[test]
    fn no_real_token_survives() {
        let fixture = include_str!("testdata/jsonl/redaction-sk-ant.jsonl");
        let out = redact_secrets(fixture);
        // The raw token bodies from the fixture — none must remain.
        assert!(!out.contains("sk-ant-short"));
        assert!(!out.contains("sk-ant-Abc123DEF456_ghiJKLxyz"));
        assert!(!out.contains("sk-ant-oat01-Abc_123-DEFghiJKL-xyz"));
        // Every occurrence of `sk-ant-` must be followed immediately
        // by the masking marker `*`, confirming no live token survived.
        for (idx, _) in out.match_indices("sk-ant-") {
            let after = &out[idx + "sk-ant-".len()..];
            assert!(
                after.starts_with('*'),
                "bare sk-ant- prefix at byte {idx} in: {out}"
            );
        }
    }

    #[test]
    fn opt_wrapper_handles_none_and_empty() {
        assert_eq!(redact_secrets_opt(None), "");
        assert_eq!(redact_secrets_opt(Some("")), "");
        assert_eq!(redact_secrets_opt(Some("clean")), "clean");
        assert_eq!(redact_secrets_opt(Some("sk-ant-a")), "sk-ant-***");
    }
}
