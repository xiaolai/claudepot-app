//! Secret masking — port of CC's `secretScanner.ts` rules.
//!
//! Applied to every string that crosses the IPC boundary: preview bodies,
//! search snippets, and all DTOs under `config_view`. The rule bank is
//! modelled after CC's 28-family rule list (plan §7.1) plus
//! `Authorization: Bearer`, PEM private-key multi-line, JWT-ish, and
//! the runtime-assembled Anthropic prefix families.
//!
//! This module is a PROFILE over `crate::secret_patterns`: the family
//! definitions live in [`crate::secret_patterns::provider_rules`]
//! (one definition, all consumers); this profile selects the full
//! bank and renders every match as `<redacted:{name}>`.
//!
//! Masking is **idempotent**: running `mask_text` twice is a no-op on
//! the second pass (all replacement strings are themselves
//! unambiguously non-secret).

use serde_json::Value;

/// Replace every match with `<redacted:{name}>`. Idempotent — the
/// replacement format never matches any of the rules again.
pub fn mask_text(input: &str) -> String {
    let mut out = input.to_string();
    for rule in crate::secret_patterns::provider_rules() {
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
        let jwt = format!(
            "eyJ{}.{}.{}",
            "a".repeat(20),
            "b".repeat(20),
            "c".repeat(20)
        );
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
        let reparsed: Value =
            serde_json::from_str(&masked).expect("masked JSON must remain parseable");
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

    /// A family defined once in `crate::secret_patterns` is picked up
    /// by this profile and rendered in its `<redacted:{name}>` style —
    /// the DigitalOcean rule has no definition anywhere in this file.
    #[test]
    fn shared_core_family_propagates_to_this_profile() {
        assert!(crate::secret_patterns::provider_rules()
            .iter()
            .any(|r| r.name == "digitalocean_token"));
        let tok = format!("dop_v1_{}", "a".repeat(64));
        let out = mask_text(&tok);
        assert!(out.contains("<redacted:digitalocean_token>"));
        assert!(!out.contains(&tok));
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
