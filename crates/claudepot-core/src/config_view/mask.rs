//! Secret masking — port of CC's `secretScanner.ts` rules.
//!
//! Applied to every string that crosses the IPC boundary: preview bodies,
//! search snippets, and all DTOs under `config_view`. The `RuleSet`
//! below is modelled after CC's 28-family rule list (plan §7.1) plus
//! `Authorization: Bearer`, PEM private-key multi-line, JWT-ish, and
//! the runtime-assembled Anthropic prefix families.
//!
//! Masking is **idempotent**: running `mask_text` twice is a no-op on
//! the second pass (all replacement strings are themselves
//! unambiguously non-secret).

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

/// A secret masking rule — name + pattern. Order-insensitive since each
/// rule independently rewrites its own matches to `<redacted:name>`.
struct Rule {
    name: &'static str,
    re: Regex,
}

fn r(name: &'static str, pat: &str) -> Rule {
    Rule {
        name,
        re: Regex::new(pat).unwrap_or_else(|_| panic!("bad secret regex: {name}")),
    }
}

static RULES: Lazy<Vec<Rule>> = Lazy::new(|| {
    let anthropic_families = ["api", "admin", "oat", "ort", "cc"];
    let mut rules: Vec<Rule> = Vec::new();

    // Anthropic runtime-assembled families (sk-ant-<family>-…). Runtime
    // assembly keeps the literal needle out of our binary so scanners
    // don't false-positive on it.
    for fam in anthropic_families {
        let pat = format!(r"sk-ant-{fam}[0-9]{{2}}-[A-Za-z0-9_\-]{{20,}}");
        rules.push(r("anthropic", Box::leak(pat.into_boxed_str())));
    }

    // AWS (prefixed first; `aws_secret_key`'s 40-char pattern runs LAST so
    // it doesn't consume more-specific prefixed tokens below).
    rules.push(r("aws_access_key", r"\bAKIA[0-9A-Z]{16}\b"));
    rules.push(r("aws_session_token", r"\bASIA[0-9A-Z]{16}\b"));
    // NOTE: aws_secret_key is pushed at the very end of this function.

    // GCP
    rules.push(r("gcp_key", r"\bAIza[0-9A-Za-z_\-]{35}\b"));
    rules.push(r("gcp_service_account", r#""type":\s*"service_account""#));

    // Azure
    rules.push(r(
        "azure_storage_key",
        r"DefaultEndpointsProtocol=https?;AccountName=[A-Za-z0-9]+;AccountKey=[A-Za-z0-9+/=]+",
    ));

    // DigitalOcean
    rules.push(r("digitalocean_token", r"\bdop_v1_[a-f0-9]{64}\b"));

    // OpenAI
    rules.push(r("openai", r"\bsk-[A-Za-z0-9]{20,}\b"));

    // HuggingFace
    rules.push(r("huggingface", r"\bhf_[A-Za-z0-9]{30,}\b"));

    // GitHub (PAT, fine-grained, app, OAuth, refresh)
    rules.push(r("github_pat_classic", r"\bghp_[A-Za-z0-9]{36,}\b"));
    rules.push(r("github_pat_fine", r"\bgithub_pat_[A-Za-z0-9_]{82,}\b"));
    rules.push(r("github_app", r"\bghs_[A-Za-z0-9]{36,}\b"));
    rules.push(r("github_oauth", r"\bgho_[A-Za-z0-9]{36,}\b"));
    rules.push(r("github_refresh", r"\bghr_[A-Za-z0-9]{36,}\b"));

    // GitLab
    rules.push(r("gitlab_pat", r"\bglpat-[A-Za-z0-9_\-]{20,}\b"));
    rules.push(r("gitlab_deploy", r"\bgldt-[A-Za-z0-9_\-]{20,}\b"));

    // Slack
    rules.push(r("slack_bot", r"\bxoxb-[0-9A-Za-z\-]{20,}\b"));
    rules.push(r("slack_user", r"\bxoxp-[0-9A-Za-z\-]{20,}\b"));
    rules.push(r("slack_webhook", r"https://hooks\.slack\.com/services/[A-Za-z0-9/_\-]+"));

    // Twilio
    rules.push(r("twilio_account_sid", r"\bAC[0-9a-f]{32}\b"));

    // SendGrid
    rules.push(r("sendgrid", r"\bSG\.[A-Za-z0-9_\-]{22}\.[A-Za-z0-9_\-]{43}\b"));

    // npm
    rules.push(r("npm_token", r"\bnpm_[A-Za-z0-9]{36,}\b"));

    // PyPI
    rules.push(r("pypi_token", r"\bpypi-AgEIc[A-Za-z0-9_\-]{80,}\b"));

    // Databricks
    rules.push(r("databricks", r"\bdapi[a-f0-9]{32}\b"));

    // HashiCorp TF
    rules.push(r("terraform_cloud", r"\b[A-Za-z0-9]{14}\.atlasv1\.[A-Za-z0-9_\-]{60,}\b"));

    // Pulumi
    rules.push(r("pulumi", r"\bpul-[A-Fa-f0-9]{40}\b"));

    // Postman
    rules.push(r("postman", r"\bPMAK-[A-Fa-f0-9]{24}-[A-Fa-f0-9]{34}\b"));

    // Grafana
    rules.push(r("grafana_api", r"\beyJrIjoi[A-Za-z0-9+/=_\-]{40,}\b"));
    rules.push(r("grafana_service_account", r"\bglsa_[A-Za-z0-9_]{32}_[A-Fa-f0-9]{8}\b"));
    rules.push(r("grafana_cloud", r"\bglc_[A-Za-z0-9+/=]{64,}\b"));

    // Sentry
    rules.push(r("sentry_auth", r"\bsntrys_[A-Za-z0-9+/=_\-]{60,}\b"));
    rules.push(r("sentry_dsn", r"https://[0-9a-f]{32}@[A-Za-z0-9.\-]+\.ingest\.sentry\.io/[0-9]+"));

    // Stripe
    rules.push(r("stripe_live_secret", r"\bsk_live_[A-Za-z0-9]{20,}\b"));
    rules.push(r("stripe_live_publishable", r"\bpk_live_[A-Za-z0-9]{20,}\b"));

    // Shopify
    rules.push(r("shopify_access", r"\bshpat_[A-Fa-f0-9]{32}\b"));
    rules.push(r("shopify_custom", r"\bshpca_[A-Fa-f0-9]{32}\b"));

    // PEM private key (multi-line header match)
    rules.push(r(
        "pem_private_key",
        r"-----BEGIN (RSA |EC |DSA |OPENSSH |PGP |)PRIVATE KEY-----[\s\S]*?-----END (RSA |EC |DSA |OPENSSH |PGP |)PRIVATE KEY-----",
    ));

    // Bearer + JWT (Claudepot additions, plan §7.1)
    rules.push(r(
        "bearer_token",
        r"(?i)Authorization:\s*Bearer\s+[A-Za-z0-9._~+/=\-]{10,}",
    ));
    rules.push(r(
        "jwt",
        r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\b",
    ));

    // AWS secret-key pattern (unprefixed 40-char) runs LAST so it never
    // swallows a more-specific prefixed token that came before it.
    rules.push(r("aws_secret_key", r"\b[A-Za-z0-9/+=]{40}\b(?:[^A-Za-z0-9/+=]|$)"));

    rules
});

/// Replace every match with `<redacted:{name}>`. Idempotent — the
/// replacement format never matches any of the rules again.
pub fn mask_text(input: &str) -> String {
    let mut out = input.to_string();
    for rule in RULES.iter() {
        out = rule
            .re
            .replace_all(&out, format!("<redacted:{}>", rule.name))
            .to_string();
    }
    out
}

/// Mask byte slices. Invalid UTF-8 is replaced; non-UTF-8 bodies can't
/// carry string secrets in CC's JSON/Markdown formats.
pub fn mask_bytes(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    mask_text(&text)
}

/// Mask a preview body, preferring JSON-aware masking when the content
/// parses cleanly. The byte-level regex path is unsafe on JSON: a rule
/// whose character class includes JSON delimiters (e.g.
/// `aws_secret_key`'s `[A-Za-z0-9/+=]{40}` matching a long path string)
/// can rewrite bytes across a closing `"`, producing an "Unterminated
/// string" parse error in the renderer on a healthy file. JSON-aware
/// masking only rewrites string *values*, never delimiters, and is
/// structurally incapable of breaking the document.
///
/// Falls back to `mask_bytes` for non-JSON bodies (markdown, plugin
/// READMEs, hook scripts) and for malformed/truncated JSON.
pub fn mask_preview_body(bytes: &[u8]) -> String {
    if let Ok(mut v) = serde_json::from_slice::<Value>(bytes) {
        mask_json(&mut v);
        if let Ok(s) = serde_json::to_string_pretty(&v) {
            return s;
        }
    }
    mask_bytes(bytes)
}

/// Recursively walk every string leaf and map value in a JSON tree.
/// Applies `mask_text` to each string. Returns a new tree — callers
/// must not keep the original after calling.
pub fn mask_json(v: &mut Value) {
    match v {
        Value::String(s) => {
            let masked = mask_text(s);
            if masked != *s {
                *s = masked;
            }
        }
        Value::Array(a) => {
            for item in a {
                mask_json(item);
            }
        }
        Value::Object(o) => {
            for (_k, val) in o.iter_mut() {
                mask_json(val);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_anthropic_runtime_family() {
        // Construct at test-time so the source literal doesn't match.
        let needle = format!("sk-ant-{}-{}", "api01", "A".repeat(50));
        let masked = mask_text(&format!("key={needle} tail"));
        assert!(masked.contains("<redacted:anthropic>"));
        assert!(!masked.contains(&needle));
    }

    #[test]
    fn masks_bearer_token() {
        let out = mask_text("Authorization: Bearer abcdefghijklmno");
        assert!(out.contains("<redacted:bearer_token>"));
    }

    #[test]
    fn masks_jwt() {
        let jwt = format!("eyJ{}.{}.{}", "a".repeat(20), "b".repeat(20), "c".repeat(20));
        let out = mask_text(&jwt);
        assert!(out.contains("<redacted:jwt>"));
    }

    #[test]
    fn masks_github_pat_classic() {
        let tok = format!("ghp_{}", "A".repeat(40));
        let out = mask_text(&tok);
        assert!(out.contains("<redacted:github_pat_classic>"));
    }

    #[test]
    fn masks_aws_access_key() {
        let out = mask_text("AKIAIOSFODNN7EXAMPLE plus padding");
        assert!(out.contains("<redacted:aws_access_key>"));
    }

    #[test]
    fn masks_slack_bot() {
        let out = mask_text("xoxb-1234-5678-AbCdEfGhIjKlMnOpQrStUv");
        assert!(out.contains("<redacted:slack_bot>"));
    }

    #[test]
    fn masks_pem_private_key() {
        let key = "-----BEGIN PRIVATE KEY-----\nAAAAC3NzaC1lZDI1\n-----END PRIVATE KEY-----";
        let out = mask_text(key);
        assert!(out.contains("<redacted:pem_private_key>"));
    }

    #[test]
    fn idempotent_second_pass_no_change() {
        let tok = format!("ghp_{}", "A".repeat(40));
        let pass1 = mask_text(&tok);
        let pass2 = mask_text(&pass1);
        assert_eq!(pass1, pass2);
    }

    #[test]
    fn json_walk_masks_nested_strings() {
        let mut v: Value = serde_json::from_str(
            r#"{
                "env": {"TOK": "__SLACK__"},
                "args": ["--token", "__GH__"]
            }"#,
        )
        .unwrap();
        let slack = "xoxp-1234-5678-ABCDEFGHIJKLMNOPQRSTUVWX";
        let gh = format!("gho_{}", "A".repeat(40));
        // Inject tokens into the tree.
        v["env"]["TOK"] = Value::String(slack.to_string());
        v["args"][1] = Value::String(gh.clone());
        mask_json(&mut v);
        let s = v.to_string();
        assert!(s.contains("<redacted:slack_user>"));
        assert!(s.contains("<redacted:github_oauth>"));
        assert!(!s.contains(slack));
        assert!(!s.contains(&gh));
    }

    #[test]
    fn plain_text_untouched() {
        let s = "this is just a note about settings.json";
        assert_eq!(mask_text(s), s);
    }

    #[test]
    fn mask_preview_body_does_not_corrupt_long_paths_in_json() {
        // Regression: byte-level `mask_bytes` would match the
        // `aws_secret_key` rule (`\b[A-Za-z0-9/+=]{40}\b…`) inside a
        // long path string and consume the closing `"`, producing an
        // "Unterminated string" parse error in the renderer on a
        // healthy file. `mask_preview_body` must round-trip such
        // values without corrupting JSON syntax.
        let original = serde_json::json!({
            "projects": {
                "claudepot-app": [
                    "/Users/joker/github/xiaolai/myprojects/booklib/src-tauri",
                    "/Users/joker/github/xiaolai/myprojects/claudepot-app/src",
                ]
            }
        });
        let bytes = serde_json::to_vec(&original).unwrap();
        let masked = mask_preview_body(&bytes);
        let reparsed: Value = serde_json::from_str(&masked)
            .expect("masked JSON must remain parseable");
        assert_eq!(reparsed, original);
    }

    #[test]
    fn mask_preview_body_still_redacts_secrets_in_json_values() {
        // JSON-aware path must mask string leaves the same way the
        // byte path does — only the delimiter handling changes.
        let tok = format!("ghp_{}", "A".repeat(40));
        let v = serde_json::json!({
            "tokens": { "github": tok.clone() },
            "notes": ["secret is", tok.clone()],
        });
        let bytes = serde_json::to_vec(&v).unwrap();
        let masked = mask_preview_body(&bytes);
        assert!(masked.contains("<redacted:github_pat_classic>"));
        assert!(!masked.contains(&tok));
    }

    #[test]
    fn mask_preview_body_falls_back_to_bytes_for_non_json() {
        // Markdown bodies don't parse as JSON — must fall through to
        // byte-level masking and still redact known patterns.
        let md = "Here is a key: AKIAIOSFODNN7EXAMPLE plus padding.";
        let masked = mask_preview_body(md.as_bytes());
        assert!(masked.contains("<redacted:aws_access_key>"));
    }

    #[test]
    fn openai_key_masked() {
        let tok = format!("sk-{}", "a".repeat(40));
        let out = mask_text(&tok);
        assert!(out.contains("<redacted:openai>"));
    }

    #[test]
    fn snapshot_bundle_never_contains_raw_needle() {
        // Canonical "IPC leak" assertion: build a JSON blob with every
        // family's needle at various nesting levels, mask, serialize.
        // The serialized bytes must never match any of the original
        // needles (plan §7.5).
        let needles: Vec<String> = vec![
            format!("sk-ant-{}-{}", "api01", "x".repeat(50)),
            format!("sk-ant-{}-{}", "admin01", "x".repeat(50)),
            format!("sk-ant-{}-{}", "oat01", "x".repeat(50)),
            format!("ghp_{}", "A".repeat(40)),
            format!("gho_{}", "A".repeat(40)),
            format!("ghs_{}", "A".repeat(40)),
            format!("glpat-{}", "a".repeat(25)),
            "xoxb-1234-5678-AbCdEfGhIjKlMnOpQrStUv".to_string(),
            "xoxp-1234-5678-AbCdEfGhIjKlMnOpQrStUv".to_string(),
            "AKIAIOSFODNN7EXAMPLE".to_string(),
            format!("npm_{}", "A".repeat(40)),
            format!("hf_{}", "A".repeat(40)),
            format!("dop_v1_{}", "a".repeat(64)),
            "dapi12345678901234567890123456789012".to_string(),
            format!("pul-{}", "a".repeat(40)),
            format!("sk-{}", "a".repeat(40)),
            format!("sk_live_{}", "A".repeat(40)),
            format!("pk_live_{}", "A".repeat(40)),
            format!("shpat_{}", "a".repeat(32)),
            format!(
                "eyJ{}.{}.{}",
                "a".repeat(30),
                "b".repeat(30),
                "c".repeat(30)
            ),
            "Authorization: Bearer fake-token-12345678".to_string(),
        ];
        let mut v = serde_json::json!({
            "top": needles.clone(),
            "mid": { "k": needles[0] },
        });
        mask_json(&mut v);
        let serialized = serde_json::to_string(&v).unwrap();
        for n in &needles {
            assert!(
                !serialized.contains(n),
                "needle leaked after masking: {n}\nserialized: {serialized}",
            );
        }
    }
}
