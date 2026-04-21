//! Redaction — the sanitizer applied at every boundary that carries
//! user-authored or model-generated text out of `claudepot-core`.
//!
//! **Families covered as of M2:**
//!   1. Anthropic keys: `sk-ant-<alnum/_/->+` (API keys + OAuth
//!      `sk-ant-oat01-*` variants). ≤12-char match fully masked;
//!      longer keeps last 4 chars so two leaks stay distinguishable.
//!   2. Authorization headers: `Authorization: Bearer <token>` and
//!      `Authorization: Basic <blob>` case-insensitive; the token
//!      body is replaced with `***`.
//!   3. Bearer JWTs: three dot-separated base64url runs (the
//!      canonical JWT shape) → `eyJ***.***.***`-style mask that
//!      preserves the prefix so readers can tell "this is a JWT."
//!   4. Key-value query/body params with sensitive names
//!      (`password`, `passwd`, `api_key`, `apikey`, `token`,
//!      `secret`, `access_token`, `refresh_token`, `authorization`).
//!      The VALUE is masked; the key name stays visible.
//!   5. Cookie headers: `Cookie: name=value; ...` and `Set-Cookie:
//!      name=value; ...`. Cookie values are masked, attribute keys
//!      (`Path`, `HttpOnly`, …) left intact for context.
//!
//! **Trust-boundary positioning.** The boundary is the IPC layer —
//! Tauri commands that emit `LiveSessionSummary` / `LiveDelta` DTOs.
//! This module is one policy that boundary calls; it is NOT a
//! complete sanitizer. Every call site that emits a user-content
//! string must invoke `redact_secrets` before crossing out.
//!
//! Extension pattern: add a new `static FAMILY_RE: Lazy<Regex>`,
//! apply it inside `redact_secrets` with a `replace_all` closure,
//! and ship a fixture pair (positive + negative) under `tests`.

use once_cell::sync::Lazy;
use regex::Regex;

/// Anthropic key family. `sk-ant-` plus one or more of
/// `[A-Za-z0-9_-]`. Matches OAuth variants (`sk-ant-oat01-...`)
/// because the prefix family is shared.
static SK_ANT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"sk-ant-[A-Za-z0-9_-]+").expect("static regex"));

/// `Authorization: Bearer <token>` and `Authorization: Basic <blob>`
/// HTTP headers. Case-insensitive on the scheme, whitespace-tolerant
/// between header name and value. Captures the token body as group 1
/// so we can replace it individually.
static AUTH_HEADER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(Authorization\s*:\s*(?:Bearer|Basic)\s+)([A-Za-z0-9._\-=/+]+)")
        .expect("static regex")
});

/// JWT-shaped tokens: three base64url-safe runs separated by dots.
/// `eyJ` is the literal ASCII prefix of any JWT whose header begins
/// `{"typ"...` or `{"alg"...`, which is essentially all of them.
/// Anchoring on `eyJ` keeps false positives away from arbitrary
/// dotted identifiers.
static JWT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"eyJ[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+")
        .expect("static regex")
});

/// Key=value pairs with a sensitive-looking key name. The delimiter
/// may be `=` (query/form) or `:` with a space (YAML/JSON-ish). The
/// key-name alternation is intentionally narrow so generic names
/// like `data` or `content` don't trigger. Matches case-insensitively
/// via the `(?i)` flag, so `PASSWORD=`, `Api_Key=`, etc. all hit.
///
/// The bare `token` keyword catches `token=...` and `AUTHORIZATION:
/// Token ...` forms the narrower Authorization regex would miss.
static SENSITIVE_KV_RE: Lazy<Regex> = Lazy::new(|| {
    // `Authorization:` is handled by the outer AUTH_HEADER_RE pass
    // specifically so we can preserve the `Authorization: Bearer ***`
    // shape; don't duplicate `auth(orization)?` here or it would
    // overwrite the structured mask with the generic one.
    Regex::new(
        r#"(?i)\b(password|passwd|api[_-]?key|access[_-]?token|refresh[_-]?token|secret|bearer|token)\s*[:=]\s*"?([^"\s&;,}]+)"?"#,
    )
    .expect("static regex")
});

/// `Cookie:` and `Set-Cookie:` headers. Captures the entire rest of
/// the line so we can mask cookie VALUES while preserving the key=
/// shape + attribute keywords (Path, HttpOnly, Secure, SameSite …)
/// which are useful context and never sensitive.
static COOKIE_HEADER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(Cookie|Set-Cookie)\s*:\s*([^\r\n]+)")
        .expect("static regex")
});

const COOKIE_ATTR_NAMES: &[&str] = &[
    "path", "domain", "expires", "max-age", "secure", "httponly",
    "samesite", "partitioned", "priority",
];

/// Length threshold below which a matched sk-ant token is masked
/// entirely rather than keeping a suffix.
const SHORT_TOKEN_THRESHOLD: usize = 12;

/// Redact every supported secret family in `text`. Fast-path skips
/// regex work when no prefix/sentinel is present (common case for
/// bulk transcript content that contains none of these patterns).
/// All sentinel probes are case-insensitive so `PASSWORD=`,
/// `AUTHORIZATION:`, and `TOKEN=` all hit the slow path.
pub fn redact_secrets(text: &str) -> String {
    let lowered_hint = text.to_ascii_lowercase();
    let needs_work = lowered_hint.contains("sk-ant-")
        || lowered_hint.contains("authorization")
        || lowered_hint.contains("cookie")
        || text.contains("eyJ")
        || lowered_hint.contains("password")
        || lowered_hint.contains("passwd")
        || lowered_hint.contains("api_key")
        || lowered_hint.contains("api-key")
        || lowered_hint.contains("apikey")
        || lowered_hint.contains("access_token")
        || lowered_hint.contains("refresh_token")
        || lowered_hint.contains("secret")
        || lowered_hint.contains("token")
        || lowered_hint.contains("bearer");
    if !needs_work {
        return text.to_string();
    }
    // Redact Authorization header values first so the inner token
    // (which might itself look JWT-shaped) is hidden before the JWT
    // pass runs. Order matters: outer-to-inner lets us avoid
    // double-masking.
    let mut out = AUTH_HEADER_RE
        .replace_all(text, |caps: &regex::Captures<'_>| {
            format!("{}***", &caps[1])
        })
        .into_owned();
    out = COOKIE_HEADER_RE
        .replace_all(&out, |caps: &regex::Captures<'_>| {
            let header = &caps[1];
            let body = &caps[2];
            format!("{}: {}", header, mask_cookie_body(body))
        })
        .into_owned();
    out = JWT_RE
        .replace_all(&out, "eyJ***.***.***")
        .into_owned();
    out = SENSITIVE_KV_RE
        .replace_all(&out, |caps: &regex::Captures<'_>| {
            // Preserve the original separator so Bash-style args like
            // `--password=foo` don't switch to `--password: ***`.
            let key = &caps[1];
            let full = &caps[0];
            let value_start = caps.get(2).map(|m| m.start()).unwrap_or(0);
            let sep_chunk = &full[key.len()..(value_start - caps.get(0).unwrap().start())];
            format!("{key}{sep_chunk}***")
        })
        .into_owned();
    SK_ANT_RE
        .replace_all(&out, |caps: &regex::Captures<'_>| {
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

/// Mask every `name=value` in a cookie body; keep attribute keywords
/// (Path, HttpOnly, …) intact for context. Called from the Cookie /
/// Set-Cookie header replacer.
fn mask_cookie_body(body: &str) -> String {
    body.split(';')
        .map(|chunk| {
            let trimmed = chunk.trim();
            if trimmed.is_empty() {
                return trimmed.to_string();
            }
            if let Some(eq) = trimmed.find('=') {
                let (k, _v) = trimmed.split_at(eq);
                // Attribute keywords (Path=/, Max-Age=3600) keep
                // their shape with masked values.
                if COOKIE_ATTR_NAMES
                    .iter()
                    .any(|a| a.eq_ignore_ascii_case(k.trim()))
                {
                    return format!("{k}=***");
                }
                format!("{k}=***")
            } else if COOKIE_ATTR_NAMES
                .iter()
                .any(|a| a.eq_ignore_ascii_case(trimmed))
            {
                // Value-less attribute like `HttpOnly`, `Secure`.
                trimmed.to_string()
            } else {
                "***".to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
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
        // OAuth access tokens share the `sk-ant-` prefix via `oat01-`.
        // Post-M2 the `token=` prefix itself triggers SENSITIVE_KV
        // masking (`token=***`), which is strictly stronger than
        // the `sk-ant-***Lxyz`-style suffix-preserving mask — the
        // raw body must not appear in either case.
        let tok = "sk-ant-oat01-Abc_123-DEFghiJKL-xyz";
        let out = redact_secrets(&format!("token={tok}"));
        assert!(!out.contains(tok));
        // Either mask form is acceptable; both hide the body.
        assert!(
            out.contains("token=***") || out.contains("sk-ant-***"),
            "expected redaction marker, got: {out}"
        );
    }

    #[test]
    fn bare_sk_ant_still_gets_suffix_mask() {
        // When there's no `token=` / `Authorization:` wrapper, the
        // sk-ant pass is the last-mile fallback and should preserve
        // the last-4 suffix so different leaks stay distinguishable.
        let out = redact_secrets(
            "stray sk-ant-Abc123DEF456_ghiJKLxyz trailing",
        );
        assert!(out.contains("sk-ant-***Lxyz"));
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

    // ── M2 expansion: Authorization header ─────────────────────────

    #[test]
    fn auth_bearer_header_masks_token() {
        let out = redact_secrets(
            r#"curl -H "Authorization: Bearer abc123XYZdef456""#,
        );
        assert!(!out.contains("abc123XYZdef456"));
        assert!(out.contains("Authorization: Bearer ***"));
    }

    #[test]
    fn auth_basic_header_masks_blob() {
        let out = redact_secrets(
            "Authorization: Basic dXNlcjpwYXNzd29yZA==",
        );
        assert!(!out.contains("dXNlcjpwYXNzd29yZA"));
        assert!(out.contains("Authorization: Basic ***"));
    }

    #[test]
    fn auth_header_case_insensitive() {
        let out = redact_secrets("authorization: bearer TOK12345");
        assert!(!out.contains("TOK12345"));
        assert!(out.to_lowercase().contains("authorization: bearer ***"));
    }

    #[test]
    fn auth_header_no_bearer_passes_through() {
        // A line mentioning 'Authorization' without the Bearer/Basic
        // scheme isn't a credential disclosure — don't rewrite.
        let input = "The Authorization documentation is at ...";
        let out = redact_secrets(input);
        assert_eq!(out, input);
    }

    // ── M2 expansion: JWT ──────────────────────────────────────────

    #[test]
    fn jwt_is_masked_to_canonical_shape() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJ1c2VyIjoiam9rZXIifQ.abcDEF-xyz_123";
        let out = redact_secrets(&format!("token={jwt}"));
        assert!(!out.contains("eyJhbGciOiJIUzI1NiJ9"));
        // M2 masks the full JWT via SENSITIVE_KV_RE (token=<body>)
        // to `***` — earlier than the JWT regex would get to it.
        // Either way the raw body must not survive.
        assert!(!out.contains(".abcDEF-xyz_123"));
    }

    #[test]
    fn standalone_jwt_without_key_prefix_is_masked() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJ1c2VyIjoiam9rZXIifQ.signature1";
        let out = redact_secrets(&format!("received: {jwt}"));
        assert!(!out.contains("eyJhbGciOiJIUzI1NiJ9"));
        assert!(out.contains("eyJ***.***.***"));
    }

    #[test]
    fn dotted_identifier_without_eyj_prefix_not_mistaken_for_jwt() {
        // Module paths look nothing like JWTs, but assert anyway
        // so future regex tweaks don't regress.
        let input = "a.b.c and foo.bar.baz are fine";
        let out = redact_secrets(input);
        assert_eq!(out, input);
    }

    // ── M2 expansion: key=value params ─────────────────────────────

    #[test]
    fn password_param_value_masked() {
        let out = redact_secrets("db-url?password=hunter2&user=joker");
        assert!(!out.contains("hunter2"));
        assert!(out.contains("password=***"));
        assert!(out.contains("user=joker"));
    }

    #[test]
    fn api_key_variants_all_caught() {
        for variant in ["api_key", "api-key", "apikey", "API_KEY"] {
            let out = redact_secrets(&format!("{variant}=XYZ-leakme-123"));
            assert!(
                !out.contains("XYZ-leakme-123"),
                "variant {variant} leaked"
            );
        }
    }

    #[test]
    fn uppercase_sentinel_words_take_the_slow_path() {
        // Every sentinel word the redactor guards must trigger on
        // both cases. Fast-path mis-case would let uppercase
        // PASSWORD= / TOKEN= slip through the whole pipeline.
        let cases = [
            "PASSWORD=hunter2",
            "TOKEN=abc123def",
            "API_KEY=XYZLeak",
            "SECRET=omgno",
        ];
        for input in &cases {
            let out = redact_secrets(input);
            assert!(
                !out.contains(&input[input.find('=').unwrap() + 1..]),
                "uppercase variant {input} leaked"
            );
        }
    }

    #[test]
    fn generic_token_key_gets_masked() {
        // The bare `token=...` key was not covered by the earlier
        // regex; the M2-review expansion added it.
        let out = redact_secrets("token=abc-DEF_leakme");
        assert!(!out.contains("abc-DEF_leakme"));
    }

    #[test]
    fn generic_field_names_untouched() {
        // Don't mask arbitrary key=value pairs — only the sensitive
        // list. Log lines would become useless otherwise.
        let input = "user=joker&count=42&status=ok";
        let out = redact_secrets(input);
        assert_eq!(out, input);
    }

    #[test]
    fn separator_style_preserved() {
        // Bash-style `--password=foo` stays `--password=***`.
        // YAML-style `password: foo` stays `password: ***`.
        let bash = redact_secrets("--password=sekret1");
        assert!(bash.contains("--password=***"));
        assert!(!bash.contains("sekret1"));
        let yaml = redact_secrets("password: sekret2");
        assert!(yaml.contains("password:") && yaml.contains("***"));
        assert!(!yaml.contains("sekret2"));
    }

    // ── M2 expansion: cookies ──────────────────────────────────────

    #[test]
    fn cookie_header_masks_values_preserves_attrs() {
        let out = redact_secrets(
            "Cookie: sid=abc123; lang=en-US; Path=/; HttpOnly",
        );
        assert!(!out.contains("abc123"));
        assert!(!out.contains("en-US"));
        assert!(out.contains("sid=***"));
        assert!(out.contains("lang=***"));
        // Attribute keywords carry their shape but mask any value.
        assert!(out.contains("Path=***"));
        assert!(out.contains("HttpOnly"));
    }

    #[test]
    fn set_cookie_header_also_masked() {
        let out = redact_secrets(
            "Set-Cookie: token=eyJabc.def.ghi; Secure; Max-Age=3600",
        );
        assert!(!out.contains("eyJabc.def.ghi"));
        assert!(out.contains("token=***"));
        assert!(out.contains("Max-Age=***"));
        assert!(out.contains("Secure"));
    }

    // ── Idempotence across families ────────────────────────────────

    #[test]
    fn m2_families_are_idempotent_when_combined() {
        let input = concat!(
            "Authorization: Bearer sk-ant-Abc123DEF456_ghiJKLxyz\n",
            "password=hunter2\n",
            "Cookie: sid=abc; Path=/\n",
            "token=eyJhbGciOiJIUzI1NiJ9.eyJ1c2VyIjoiam9rZXIifQ.sig1\n",
        );
        let once = redact_secrets(input);
        let twice = redact_secrets(&once);
        assert_eq!(once, twice, "not idempotent across M2 families");
        // And every raw secret body is absent:
        for leak in [
            "sk-ant-Abc123DEF456_ghiJKLxyz",
            "hunter2",
            "sid=abc",
            "eyJhbGciOiJIUzI1NiJ9",
        ] {
            assert!(!once.contains(leak), "{leak} leaked through");
        }
    }
}
