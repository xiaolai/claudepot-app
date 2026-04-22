//! Composable text redaction used by session search snippets and
//! session export.
//!
//! Three independent axes:
//!   · Anthropic tokens    — always-on by default, regex bank
//!   · Path strategy       — off / relative-to-root / hash
//!   · Opt-in clauses      — emails, env assignments, custom regex
//!
//! All clauses are additive and scan the input once each. Output is
//! byte-identical outside the matched regions.

use std::path::PathBuf;

/// How to handle absolute filesystem paths in the text.
#[derive(Debug, Clone)]
pub enum PathStrategy {
    /// Leave paths untouched.
    Off,
    /// Rewrite any path under `root` to its relative form.
    Relative { root: PathBuf },
    /// Replace every absolute path with a short hash token.
    Hash,
}

impl Default for PathStrategy {
    fn default() -> Self {
        PathStrategy::Off
    }
}

#[derive(Debug, Clone)]
pub struct RedactionPolicy {
    /// Mask `sk-ant-*` tokens to `sk-ant-***<last4>`. Default `true`.
    pub anthropic_keys: bool,
    /// Path rewrite strategy.
    pub paths: PathStrategy,
    /// Mask email-like strings with `<email-redacted>`.
    pub emails: bool,
    /// Drop lines that look like `FOO=bar` environment assignments.
    pub env_assignments: bool,
    /// Extra user-supplied regex patterns (compiled once; each match is
    /// replaced with `<redacted>`).
    pub custom_regex: Vec<String>,
}

impl Default for RedactionPolicy {
    fn default() -> Self {
        Self {
            anthropic_keys: true,
            paths: PathStrategy::default(),
            emails: false,
            env_assignments: false,
            custom_regex: Vec::new(),
        }
    }
}

/// Apply the policy to `input`. Idempotent: `apply(apply(x, p), p) ==
/// apply(x, p)` for every policy.
pub fn apply(input: &str, p: &RedactionPolicy) -> String {
    let mut s: String = input.to_string();
    if p.anthropic_keys {
        s = crate::session_export::redact_secrets(&s);
    }
    s = apply_paths(&s, &p.paths);
    if p.emails {
        s = apply_emails(&s);
    }
    if p.env_assignments {
        s = apply_env_lines(&s);
    }
    for pat in &p.custom_regex {
        s = apply_custom(&s, pat);
    }
    s
}

// ---------------------------------------------------------------------------
// Individual clauses
// ---------------------------------------------------------------------------

fn apply_paths(input: &str, strat: &PathStrategy) -> String {
    match strat {
        PathStrategy::Off => input.to_string(),
        PathStrategy::Relative { root } => {
            let needle = root.to_string_lossy().to_string();
            if needle.is_empty() {
                return input.to_string();
            }
            input.replace(&needle, "<root>")
        }
        PathStrategy::Hash => scan_paths_and_hash(input),
    }
}

/// Replace every absolute path (starts with `/` or `X:\`) with a
/// stable hash. Intentionally conservative: we scan whitespace-
/// separated tokens so we don't rewrite substrings inside a larger
/// identifier.
fn scan_paths_and_hash(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut iter = input.split_inclusive(|c: char| c.is_whitespace() || c == '"');
    for tok in iter.by_ref() {
        // Split the whitespace/punctuation tail off `tok` so we only
        // hash the body.
        let (body, tail) = split_trailing_punct(tok);
        if looks_like_abs_path(body) {
            out.push_str(&format!("<path:{}>", short_hash(body)));
            out.push_str(tail);
        } else {
            out.push_str(tok);
        }
    }
    out
}

fn split_trailing_punct(tok: &str) -> (&str, &str) {
    let end = tok
        .rfind(|c: char| !(c.is_whitespace() || c == '"' || c == ',' || c == ')'))
        .map(|i| i + tok[i..].chars().next().unwrap().len_utf8())
        .unwrap_or(tok.len());
    (&tok[..end], &tok[end..])
}

fn looks_like_abs_path(tok: &str) -> bool {
    let bytes = tok.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    // POSIX absolute
    if bytes[0] == b'/' && bytes.len() > 1 {
        return true;
    }
    // Windows drive-letter, e.g. `C:\…`.
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' {
        return true;
    }
    // Windows UNC `\\server\…`
    if bytes.len() >= 2 && bytes[0] == b'\\' && bytes[1] == b'\\' {
        return true;
    }
    false
}

fn short_hash(s: &str) -> String {
    // FNV-1a — tiny, deterministic, no dep.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", (h & 0xffff_ffff) as u32)
}

fn apply_emails(input: &str) -> String {
    // Minimal scanner — no dep on `regex`. Walk whitespace-separated
    // tokens; if a token contains `@` with letters/digits/dots on
    // both sides and a top-level domain, mask it.
    let mut out = String::with_capacity(input.len());
    for tok in input.split_inclusive(|c: char| c.is_whitespace()) {
        let (body, tail) = split_trailing_punct(tok);
        if looks_like_email(body) {
            out.push_str("<email-redacted>");
            out.push_str(tail);
        } else {
            out.push_str(tok);
        }
    }
    out
}

fn looks_like_email(tok: &str) -> bool {
    let at = match tok.find('@') {
        Some(i) => i,
        None => return false,
    };
    let (local, domain_full) = tok.split_at(at);
    let domain = &domain_full[1..];
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    if !domain.contains('.') {
        return false;
    }
    local
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '.' | '+' | '-' | '_'))
        && domain
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '-'))
}

fn apply_env_lines(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for line in input.split_inclusive('\n') {
        if looks_like_env_line(line.trim_end_matches('\n')) {
            // Keep the newline for layout but drop the body.
            if line.ends_with('\n') {
                out.push_str("<env-redacted>\n");
            } else {
                out.push_str("<env-redacted>");
            }
        } else {
            out.push_str(line);
        }
    }
    out
}

fn looks_like_env_line(line: &str) -> bool {
    // `FOO=bar` where FOO is an uppercase identifier — typical env var.
    let t = line.trim();
    if !t.contains('=') {
        return false;
    }
    let (name, _) = t.split_at(t.find('=').unwrap());
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
        && name.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)
}

/// Dumb literal substring replacement as the "custom regex" fallback
/// to avoid adding a `regex` dependency for this first pass. Users
/// who need a real regex can compose multiple literals; a real engine
/// can land later without changing the policy surface.
fn apply_custom(input: &str, pattern: &str) -> String {
    if pattern.is_empty() {
        return input.to_string();
    }
    input.replace(pattern, "<redacted>")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_masks_anthropic_keys_only() {
        let p = RedactionPolicy::default();
        let input = "leaked sk-ant-oat01-AbCd1234WxYz and carry on";
        let out = apply(input, &p);
        assert!(out.contains("sk-ant-***"));
        assert!(!out.contains("AbCd1234"));
        // Non-token text is unchanged byte-for-byte.
        assert!(out.contains("leaked"));
        assert!(out.contains("carry on"));
    }

    #[test]
    fn apply_is_idempotent() {
        let p = RedactionPolicy {
            emails: true,
            paths: PathStrategy::Hash,
            custom_regex: vec!["secret-token".into()],
            ..RedactionPolicy::default()
        };
        let input =
            "reach me at me@example.com about /Users/a/b.jsonl and sk-ant-oat01-AbCdWxYz and secret-token x";
        let once = apply(input, &p);
        let twice = apply(&once, &p);
        assert_eq!(once, twice);
    }

    #[test]
    fn path_relative_rewrites_under_root_only() {
        let p = RedactionPolicy {
            paths: PathStrategy::Relative {
                root: PathBuf::from("/Users/joker"),
            },
            anthropic_keys: false,
            ..RedactionPolicy::default()
        };
        let input = "edit /Users/joker/project/a.txt and /etc/hosts";
        let out = apply(input, &p);
        assert!(out.contains("<root>/project/a.txt"));
        assert!(out.contains("/etc/hosts"));
    }

    #[test]
    fn path_hash_replaces_abs_paths_on_unix_and_windows_shapes() {
        let p = RedactionPolicy {
            paths: PathStrategy::Hash,
            anthropic_keys: false,
            ..RedactionPolicy::default()
        };
        let input = "unix /Users/joker/a.jsonl drive C:\\Users\\joker\\a and unc \\\\server\\share\\x";
        let out = apply(input, &p);
        assert!(out.contains("<path:"));
        assert!(!out.contains("/Users/joker"));
        assert!(!out.contains("C:\\Users\\joker\\a"));
        assert!(!out.contains("\\\\server\\share\\x"));
    }

    #[test]
    fn emails_clause_masks_only_email_like_tokens() {
        let p = RedactionPolicy {
            emails: true,
            anthropic_keys: false,
            ..RedactionPolicy::default()
        };
        let out = apply("email me@example.com or no-email-here", &p);
        assert!(out.contains("<email-redacted>"));
        assert!(out.contains("no-email-here"));
    }

    #[test]
    fn env_clause_drops_matching_lines_only() {
        let p = RedactionPolicy {
            env_assignments: true,
            anthropic_keys: false,
            ..RedactionPolicy::default()
        };
        let input = "normal line\nFOO_BAR=secret\nprose = not env\n";
        let out = apply(input, &p);
        assert!(out.contains("normal line"));
        assert!(out.contains("<env-redacted>"));
        assert!(out.contains("prose = not env"));
        assert!(!out.contains("FOO_BAR=secret"));
    }

    #[test]
    fn custom_regex_is_literal_substring_replacement() {
        let p = RedactionPolicy {
            custom_regex: vec!["acme-key-".into()],
            anthropic_keys: false,
            ..RedactionPolicy::default()
        };
        let out = apply("leaked acme-key-42 stays", &p);
        assert!(out.contains("<redacted>42"));
    }

    #[test]
    fn byte_identical_outside_matches() {
        let p = RedactionPolicy::default();
        let input = "completely clean prose — no secrets here.";
        assert_eq!(apply(input, &p), input);
    }

    #[test]
    fn unicode_paths_do_not_break_scanner() {
        let p = RedactionPolicy {
            paths: PathStrategy::Hash,
            anthropic_keys: false,
            ..RedactionPolicy::default()
        };
        let input = "edit /Users/héllo/año.jsonl today";
        let out = apply(input, &p);
        assert!(out.contains("<path:"));
    }
}
