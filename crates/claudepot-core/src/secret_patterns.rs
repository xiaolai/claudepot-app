//! Secret token-family definitions — the single home for "what IS a
//! secret" across every masking engine in the crate.
//!
//! Three profiles consume these definitions. Each keeps its own
//! public API, its own masking STYLE, and its own selection of
//! families — the profile layer decides how to *render* a match;
//! this module decides what *is* a match:
//!
//!   · [`crate::session_live::redact`] — the live-session IPC
//!     boundary. Selects: sk-ant (generic regex), Authorization
//!     headers, loose JWT, sensitive key=value params, cookies.
//!     Renders in-place masks (`sk-ant-***<last4>`,
//!     `Authorization: Bearer ***`, `password=***`, …).
//!   · [`crate::redaction`] — the export / MCP / shared-memory
//!     policy engine. Selects: sk-ant (linear-scanner semantics via
//!     [`sk_ant_scan_ranges`]). Renders `sk-ant-***<last4>`.
//!   · [`crate::config_view::mask`] — the Config viewer + MCP
//!     emission broad bank. Selects: the full [`provider_rules`]
//!     bank. Renders `<redacted:{name}>`.
//!
//! **Adding a new token family here propagates to session-live
//! redaction, export/MCP redaction, and config masking; add a test
//! in each consumer profile.**
//!
//! ## Documented divergences (intentional, behavior-locked)
//!
//! The three engines predate this module and genuinely disagree on
//! some boundary cases. Each profile preserves its historical
//! answer — do not "fix" these silently; every consumer's test
//! suite locks its current behavior byte-for-byte:
//!
//!   · **sk-ant boundaries.** [`SK_ANT_GENERIC_RE`] requires ≥1
//!     body char after `sk-ant-`. [`sk_ant_scan_ranges`] (the
//!     linear scanner used by `redaction`) additionally matches a
//!     bare `sk-ant-` prefix with an *empty* body, and skips
//!     already-masked `sk-ant-***<suffix>` runs so re-masking is
//!     idempotent. The provider bank's `anthropic` rules are
//!     stricter still: they require a known family infix
//!     (`api|admin|oat|ort|cc` + two digits) and a ≥20-char body.
//!   · **Authorization headers.** [`AUTH_HEADER_CAPTURE_RE`]
//!     (live) covers `Bearer` and `Basic`, tolerates whitespace
//!     before the colon, and has no minimum token length. The
//!     bank's `bearer_token` rule covers `Bearer` only, requires
//!     the colon to follow `Authorization` immediately, and needs
//!     a ≥10-char token.
//!   · **JWTs.** [`JWT_LOOSE_RE`] (live) accepts ≥1-char segments
//!     with no word boundaries; the bank's `jwt` rule requires
//!     ≥10-char segments inside `\b…\b`.

use once_cell::sync::Lazy;
use regex::Regex;

// ---------------------------------------------------------------------------
// Family: Anthropic sk-ant tokens
// ---------------------------------------------------------------------------

/// The shared Anthropic key prefix. Every sk-ant matcher variant in
/// this module builds on this prefix plus the `[A-Za-z0-9_-]` body
/// character class.
pub const SK_ANT_PREFIX: &str = "sk-ant-";

/// Anthropic key family, generic regex form: `sk-ant-` plus one or
/// more of `[A-Za-z0-9_-]`. Matches OAuth variants
/// (`sk-ant-oat01-...`) because the prefix family is shared. Used by
/// the session-live profile.
pub static SK_ANT_GENERIC_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"sk-ant-[A-Za-z0-9_-]+").expect("static regex"));

fn is_sk_ant_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')
}

/// Anthropic key family, linear-scanner form: return the byte ranges
/// of every raw `sk-ant-…` token in `text`, in order. Used by the
/// `redaction` profile (export / MCP / shared-memory).
///
/// Scanner semantics (divergent from [`SK_ANT_GENERIC_RE`], see the
/// module docs): a bare `sk-ant-` prefix with an empty body IS a
/// match, and an existing `sk-ant-***<suffix>` mask (the `*`
/// sentinel immediately after the prefix) is skipped so masking
/// stays idempotent. Every returned range starts and ends on an
/// ASCII byte, so it is always a valid `str` slice boundary.
pub fn sk_ant_scan_ranges(text: &str) -> Vec<(usize, usize)> {
    if !text.contains(SK_ANT_PREFIX) {
        return Vec::new();
    }
    let needle = SK_ANT_PREFIX.as_bytes();
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        match find_from(bytes, cursor, needle) {
            Some(start) => {
                let tok_end = token_end(bytes, start);
                // Idempotency: the mask form is `sk-ant-***<last4>`,
                // so the `*` sentinel always sits immediately after
                // the `sk-ant-` prefix, with no token chars in
                // between. If any token chars were consumed before
                // the `*`, this is a real `sk-ant-realToken*` —
                // report it instead of skipping.
                let prefix_end = start + needle.len();
                if tok_end == prefix_end && tok_end < bytes.len() && bytes[tok_end] == b'*' {
                    cursor = skip_existing_mask(bytes, tok_end);
                    continue;
                }
                ranges.push((start, tok_end));
                cursor = tok_end;
            }
            None => break,
        }
    }
    ranges
}

fn skip_existing_mask(bytes: &[u8], from: usize) -> usize {
    // Consume the `*` run.
    let mut i = from;
    while i < bytes.len() && bytes[i] == b'*' {
        i += 1;
    }
    // Then the optional 4-char last4 suffix (alnum / - / _).
    while i < bytes.len() && is_sk_ant_token_byte(bytes[i]) {
        i += 1;
    }
    i
}

fn find_from(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() || from > hay.len() - needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn token_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && is_sk_ant_token_byte(bytes[i]) {
        i += 1;
    }
    i
}

// ---------------------------------------------------------------------------
// Family: Authorization headers (live capture form)
// ---------------------------------------------------------------------------

/// `Authorization: Bearer <token>` and `Authorization: Basic <blob>`
/// HTTP headers. Case-insensitive on the scheme, whitespace-tolerant
/// between header name and value. Captures the header-plus-scheme as
/// group 1 and the token body as group 2 so consumers can replace
/// the token individually. The provider bank carries the stricter
/// `bearer_token` variant (see module docs).
pub static AUTH_HEADER_CAPTURE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(Authorization\s*:\s*(?:Bearer|Basic)\s+)([A-Za-z0-9._\-=/+]+)")
        .expect("static regex")
});

// ---------------------------------------------------------------------------
// Family: JWTs (loose form)
// ---------------------------------------------------------------------------

/// JWT-shaped tokens: three base64url-safe runs separated by dots.
/// `eyJ` is the literal ASCII prefix of any JWT whose header begins
/// `{"typ"...` or `{"alg"...`, which is essentially all of them.
/// Anchoring on `eyJ` keeps false positives away from arbitrary
/// dotted identifiers. The provider bank carries the stricter `jwt`
/// variant (see module docs).
pub static JWT_LOOSE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"eyJ[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+").expect("static regex")
});

// ---------------------------------------------------------------------------
// Family: sensitive key=value params
// ---------------------------------------------------------------------------

/// Key=value pairs with a sensitive-looking key name. The delimiter
/// may be `=` (query/form) or `:` with a space (YAML/JSON-ish). The
/// key-name alternation is intentionally narrow so generic names
/// like `data` or `content` don't trigger. Matches case-insensitively
/// via the `(?i)` flag, so `PASSWORD=`, `Api_Key=`, etc. all hit.
///
/// The bare `token` keyword catches `token=...` and `AUTHORIZATION:
/// Token ...` forms the narrower Authorization regex would miss.
pub static SENSITIVE_KV_RE: Lazy<Regex> = Lazy::new(|| {
    // `Authorization:` is handled by the AUTH_HEADER_CAPTURE_RE pass
    // specifically so consumers can preserve the `Authorization:
    // Bearer ***` shape; don't duplicate `auth(orization)?` here or
    // it would overwrite the structured mask with the generic one.
    //
    // Explicit OAuth variants (`client_secret`, `id_token`, `client_id`)
    // are listed before the bare `secret`/`token` keywords because `\b`
    // does NOT trip between `_` and a letter — both are word chars in
    // regex — so `\bsecret` would miss `client_secret=…`. Naming the
    // compound forms in their own alternatives sidesteps that.
    Regex::new(
        r#"(?i)\b(password|passwd|api[_-]?key|access[_-]?token|refresh[_-]?token|client[_-]?secret|client[_-]?id|id[_-]?token|secret|bearer|token)\s*[:=]\s*"?([^"\s&;,}]+)"?"#,
    )
    .expect("static regex")
});

// ---------------------------------------------------------------------------
// Family: Cookie / Set-Cookie headers
// ---------------------------------------------------------------------------

/// `Cookie:` and `Set-Cookie:` headers. Captures the header name as
/// group 1 and the entire rest of the line as group 2 so consumers
/// can mask cookie VALUES while preserving the key= shape +
/// attribute keywords (Path, HttpOnly, Secure, SameSite …) which are
/// useful context and never sensitive.
pub static COOKIE_HEADER_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(Cookie|Set-Cookie)\s*:\s*([^\r\n]+)").expect("static regex"));

const COOKIE_ATTR_NAMES: &[&str] = &[
    "path",
    "domain",
    "expires",
    "max-age",
    "secure",
    "httponly",
    "samesite",
    "partitioned",
    "priority",
];

/// True when `s` (case-insensitive) is a cookie *attribute* keyword
/// (`Path`, `HttpOnly`, `Max-Age`, …) rather than a cookie name —
/// attributes carry layout context, never secret values.
pub fn is_cookie_attr_name(s: &str) -> bool {
    COOKIE_ATTR_NAMES.iter().any(|a| a.eq_ignore_ascii_case(s))
}

// ---------------------------------------------------------------------------
// Family bank: broad provider rules (port of CC's secretScanner.ts)
// ---------------------------------------------------------------------------

/// A named secret rule — name + pattern. Order-insensitive for
/// rendering since each rule independently rewrites its own matches,
/// but the *bank order* matters: `aws_secret_key`'s unprefixed
/// 40-char pattern runs LAST so it never swallows a more-specific
/// prefixed token (see [`provider_rules`]).
pub struct NamedRule {
    pub name: &'static str,
    pub re: Regex,
}

fn rule(name: &'static str, pat: &str) -> NamedRule {
    NamedRule {
        name,
        re: Regex::new(pat).unwrap_or_else(|_| panic!("bad secret regex: {name}")),
    }
}

static PROVIDER_RULES: Lazy<Vec<NamedRule>> = Lazy::new(|| {
    let anthropic_families = ["api", "admin", "oat", "ort", "cc"];
    let mut rules: Vec<NamedRule> = Vec::new();

    // Anthropic runtime-assembled families (sk-ant-<family>-…). Runtime
    // assembly keeps the literal needle out of our binary so scanners
    // don't false-positive on it.
    for fam in anthropic_families {
        let pat = format!(r"sk-ant-{fam}[0-9]{{2}}-[A-Za-z0-9_\-]{{20,}}");
        rules.push(rule("anthropic", Box::leak(pat.into_boxed_str())));
    }

    // AWS (prefixed first; `aws_secret_key`'s 40-char pattern runs LAST so
    // it doesn't consume more-specific prefixed tokens below).
    rules.push(rule("aws_access_key", r"\bAKIA[0-9A-Z]{16}\b"));
    rules.push(rule("aws_session_token", r"\bASIA[0-9A-Z]{16}\b"));
    // NOTE: aws_secret_key is pushed at the very end of this function.

    // GCP
    rules.push(rule("gcp_key", r"\bAIza[0-9A-Za-z_\-]{35}\b"));
    rules.push(rule(
        "gcp_service_account",
        r#""type":\s*"service_account""#,
    ));

    // Azure
    rules.push(rule(
        "azure_storage_key",
        r"DefaultEndpointsProtocol=https?;AccountName=[A-Za-z0-9]+;AccountKey=[A-Za-z0-9+/=]+",
    ));

    // DigitalOcean
    rules.push(rule("digitalocean_token", r"\bdop_v1_[a-f0-9]{64}\b"));

    // OpenAI
    rules.push(rule("openai", r"\bsk-[A-Za-z0-9]{20,}\b"));

    // HuggingFace
    rules.push(rule("huggingface", r"\bhf_[A-Za-z0-9]{30,}\b"));

    // GitHub (PAT, fine-grained, app, OAuth, refresh)
    rules.push(rule("github_pat_classic", r"\bghp_[A-Za-z0-9]{36,}\b"));
    rules.push(rule("github_pat_fine", r"\bgithub_pat_[A-Za-z0-9_]{82,}\b"));
    rules.push(rule("github_app", r"\bghs_[A-Za-z0-9]{36,}\b"));
    rules.push(rule("github_oauth", r"\bgho_[A-Za-z0-9]{36,}\b"));
    rules.push(rule("github_refresh", r"\bghr_[A-Za-z0-9]{36,}\b"));

    // GitLab
    rules.push(rule("gitlab_pat", r"\bglpat-[A-Za-z0-9_\-]{20,}\b"));
    rules.push(rule("gitlab_deploy", r"\bgldt-[A-Za-z0-9_\-]{20,}\b"));

    // Slack
    rules.push(rule("slack_bot", r"\bxoxb-[0-9A-Za-z\-]{20,}\b"));
    rules.push(rule("slack_user", r"\bxoxp-[0-9A-Za-z\-]{20,}\b"));
    rules.push(rule(
        "slack_webhook",
        r"https://hooks\.slack\.com/services/[A-Za-z0-9/_\-]+",
    ));

    // Twilio
    rules.push(rule("twilio_account_sid", r"\bAC[0-9a-f]{32}\b"));

    // SendGrid
    rules.push(rule(
        "sendgrid",
        r"\bSG\.[A-Za-z0-9_\-]{22}\.[A-Za-z0-9_\-]{43}\b",
    ));

    // npm
    rules.push(rule("npm_token", r"\bnpm_[A-Za-z0-9]{36,}\b"));

    // PyPI
    rules.push(rule("pypi_token", r"\bpypi-AgEIc[A-Za-z0-9_\-]{80,}\b"));

    // Databricks
    rules.push(rule("databricks", r"\bdapi[a-f0-9]{32}\b"));

    // HashiCorp TF
    rules.push(rule(
        "terraform_cloud",
        r"\b[A-Za-z0-9]{14}\.atlasv1\.[A-Za-z0-9_\-]{60,}\b",
    ));

    // Pulumi
    rules.push(rule("pulumi", r"\bpul-[A-Fa-f0-9]{40}\b"));

    // Postman
    rules.push(rule("postman", r"\bPMAK-[A-Fa-f0-9]{24}-[A-Fa-f0-9]{34}\b"));

    // Grafana
    rules.push(rule("grafana_api", r"\beyJrIjoi[A-Za-z0-9+/=_\-]{40,}\b"));
    rules.push(rule(
        "grafana_service_account",
        r"\bglsa_[A-Za-z0-9_]{32}_[A-Fa-f0-9]{8}\b",
    ));
    rules.push(rule("grafana_cloud", r"\bglc_[A-Za-z0-9+/=]{64,}\b"));

    // Sentry
    rules.push(rule("sentry_auth", r"\bsntrys_[A-Za-z0-9+/=_\-]{60,}\b"));
    rules.push(rule(
        "sentry_dsn",
        r"https://[0-9a-f]{32}@[A-Za-z0-9.\-]+\.ingest\.sentry\.io/[0-9]+",
    ));

    // Stripe
    rules.push(rule("stripe_live_secret", r"\bsk_live_[A-Za-z0-9]{20,}\b"));
    rules.push(rule(
        "stripe_live_publishable",
        r"\bpk_live_[A-Za-z0-9]{20,}\b",
    ));

    // Shopify
    rules.push(rule("shopify_access", r"\bshpat_[A-Fa-f0-9]{32}\b"));
    rules.push(rule("shopify_custom", r"\bshpca_[A-Fa-f0-9]{32}\b"));

    // PEM private key (multi-line header match)
    rules.push(rule(
        "pem_private_key",
        r"-----BEGIN (RSA |EC |DSA |OPENSSH |PGP |)PRIVATE KEY-----[\s\S]*?-----END (RSA |EC |DSA |OPENSSH |PGP |)PRIVATE KEY-----",
    ));

    // Bearer + JWT (Claudepot additions, plan §7.1) — the strict
    // variants; the live profile uses AUTH_HEADER_CAPTURE_RE /
    // JWT_LOOSE_RE above (see the divergence docs).
    rules.push(rule(
        "bearer_token",
        r"(?i)Authorization:\s*Bearer\s+[A-Za-z0-9._~+/=\-]{10,}",
    ));
    rules.push(rule(
        "jwt",
        r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\b",
    ));

    // AWS secret-key pattern (unprefixed 40-char) runs LAST so it never
    // swallows a more-specific prefixed token that came before it.
    rules.push(rule(
        "aws_secret_key",
        r"\b[A-Za-z0-9/+=]{40}\b(?:[^A-Za-z0-9/+=]|$)",
    ));

    rules
});

/// The broad provider bank, in application order. `anthropic` rules
/// lead; `aws_secret_key` MUST stay last (its unprefixed 40-char
/// pattern would otherwise swallow more-specific prefixed tokens).
pub fn provider_rules() -> &'static [NamedRule] {
    PROVIDER_RULES.as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── sk-ant divergence locks ────────────────────────────────────

    #[test]
    fn generic_re_requires_at_least_one_body_char() {
        assert!(!SK_ANT_GENERIC_RE.is_match("sk-ant-"));
        assert!(SK_ANT_GENERIC_RE.is_match("sk-ant-x"));
    }

    #[test]
    fn scanner_matches_empty_body_prefix() {
        // The linear scanner (redaction profile) reports a bare
        // `sk-ant-` prefix; the generic regex would not.
        let ranges = sk_ant_scan_ranges("sk-ant- next");
        assert_eq!(ranges, vec![(0, 7)]);
    }

    #[test]
    fn scanner_skips_existing_mask_runs() {
        let text = "sk-ant-***Lxyz and sk-ant-fresh123";
        let ranges = sk_ant_scan_ranges(text);
        assert_eq!(ranges, vec![(19, 34)]);
        assert_eq!(&text[19..34], "sk-ant-fresh123");
    }

    #[test]
    fn scanner_range_bounds_are_char_boundaries() {
        let text = "héllo sk-ant-abc año";
        for (s, e) in sk_ant_scan_ranges(text) {
            assert!(text.is_char_boundary(s) && text.is_char_boundary(e));
            assert!(text[s..e].starts_with(SK_ANT_PREFIX));
        }
    }

    // ── auth-header / JWT divergence locks ─────────────────────────

    #[test]
    fn live_auth_header_covers_basic_but_strict_bank_rule_does_not() {
        let basic = "Authorization: Basic dXNlcjpwYXNzd29yZA==";
        assert!(AUTH_HEADER_CAPTURE_RE.is_match(basic));
        let strict = provider_rules()
            .iter()
            .find(|r| r.name == "bearer_token")
            .unwrap();
        assert!(!strict.re.is_match(basic));
    }

    #[test]
    fn loose_jwt_matches_short_segments_strict_does_not() {
        let short = "eyJab.cd.ef";
        assert!(JWT_LOOSE_RE.is_match(short));
        let strict = provider_rules().iter().find(|r| r.name == "jwt").unwrap();
        assert!(!strict.re.is_match(short));
    }

    // ── provider bank invariants ───────────────────────────────────

    #[test]
    fn bank_leads_with_anthropic_and_ends_with_aws_secret_key() {
        let rules = provider_rules();
        assert_eq!(rules.first().map(|r| r.name), Some("anthropic"));
        assert_eq!(rules.last().map(|r| r.name), Some("aws_secret_key"));
    }

    #[test]
    fn cookie_attr_predicate_is_case_insensitive() {
        assert!(is_cookie_attr_name("HttpOnly"));
        assert!(is_cookie_attr_name("max-age"));
        assert!(!is_cookie_attr_name("sid"));
    }
}
