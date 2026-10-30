//! A provider's error response → an Anthropic error body.
//!
//! clai relays the provider's message, redacting what looks like a key
//! (`clcd-upstream-error.jq`). The relay does not: upstream text never
//! reaches a client, a log or an event (plan §5.6), so redaction has
//! nothing to do (reviewed E11). The body is read — bounded — for one
//! signal only: whether the provider says the context window was
//! exceeded. CC recognises `prompt is too long` and compacts, so that
//! case keeps clai's prefix, followed by fixed text.
//!
//! The status passes through, so CC's own handling of 429 and 400
//! applies; `Retry-After` passes through with it.

use std::sync::LazyLock;

use axum::http::StatusCode;
use regex::Regex;
use serde_json::{json, Value};

/// How much of an error body is read.
pub const MAX_ERROR_BODY: usize = 64 * 1024;

/// Overflow wording, never a bare mention of the context: a body that
/// names "context window" in a configuration error ("context window
/// option is not supported") is not an overflow, and calling it one makes
/// CC compact a conversation that was never too long. Each arm is a
/// provider's own phrasing — OpenAI's code and its Chat and Responses
/// messages, xAI's prompt length, Gemini's token count, llama.cpp's
/// context size, Mistral's "too large for model".
static CONTEXT: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)context_length_exceeded",
        r"|maximum (context|prompt) length",
        r"|context (length|window|size|limit) (was |is |has been )?exceeded",
        r"|exceeds? (the |your )?(available |maximum )?context",
        r"|input token count.{0,40}exceeds",
        r"|too many tokens",
        r"|prompt is too long",
    ))
    .ok()
});

pub const CONTEXT_MESSAGE: &str =
    "prompt is too long: the upstream provider's context window was exceeded";

fn says_context_exceeded(body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(&body[..body.len().min(MAX_ERROR_BODY)]);
    CONTEXT.as_ref().is_some_and(|re| re.is_match(&text))
}

/// The Anthropic error type and fixed message for `status`.
pub fn classify(status: StatusCode, body: &[u8]) -> (&'static str, &'static str) {
    match status.as_u16() {
        429 => (
            "rate_limit_error",
            "The upstream provider's rate or usage limit was reached.",
        ),
        401 | 403 => (
            "authentication_error",
            "The upstream provider refused the relay's credential.",
        ),
        400..=499 if says_context_exceeded(body) => ("invalid_request_error", CONTEXT_MESSAGE),
        400..=499 => (
            "invalid_request_error",
            "The upstream provider rejected the request.",
        ),
        _ => ("api_error", "The upstream provider returned an error."),
    }
}

/// The body sent to the client.
pub fn anthropic_error(status: StatusCode, body: &[u8]) -> Value {
    let (kind, message) = classify(status, body);
    json!({"type": "error", "error": {"type": kind, "message": message}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_type_follows_the_status_and_the_text_is_never_the_providers() {
        let secret = br#"{"error":{"message":"Invalid key sk-proj-abcdef0123456789"}}"#;
        for (status, kind) in [
            (429, "rate_limit_error"),
            (401, "authentication_error"),
            (403, "authentication_error"),
            (404, "invalid_request_error"),
            (500, "api_error"),
            (503, "api_error"),
        ] {
            let v = anthropic_error(StatusCode::from_u16(status).unwrap(), secret);
            assert_eq!(v["error"]["type"], kind, "{status}");
            let text = v.to_string();
            assert!(!text.contains("sk-proj"), "{status}: {text}");
            assert!(!text.contains("Invalid key"), "{status}: {text}");
        }
    }

    #[test]
    fn a_context_overflow_keeps_the_prefix_cc_compacts_on() {
        for body in [
            &br#"{"error":{"message":"Maximum context length exceeded"}}"#[..],
            br#"{"error":{"code":"context_length_exceeded","message":"x"}}"#,
            br#"{"error":{"message":"This model's maximum context length is 128000 tokens. However, your messages resulted in 130000 tokens."}}"#,
            br#"{"error":{"message":"Your input exceeds the context window of this model. Please adjust your input and try again."}}"#,
            br#"{"error":"This model's maximum prompt length is 131072 but the request contains 140000 tokens."}"#,
            br#"{"error":{"message":"The input token count (1200000) exceeds the maximum number of tokens allowed (1048576)."}}"#,
            br#"{"error":{"message":"the request exceeds the available context size, try increasing it"}}"#,
            br#"too many tokens in request"#,
        ] {
            let v = anthropic_error(StatusCode::BAD_REQUEST, body);
            assert_eq!(v["error"]["message"], CONTEXT_MESSAGE);
            assert!(CONTEXT_MESSAGE.starts_with("prompt is too long: "));
        }
        // Only a 4xx is a context overflow.
        let v = anthropic_error(StatusCode::BAD_GATEWAY, b"context length exceeded");
        assert_eq!(v["error"]["type"], "api_error");
    }

    /// A 4xx that only mentions the context is the provider rejecting the
    /// request, not an overflow: CC must not compact on it.
    #[test]
    fn a_context_mention_that_is_not_an_overflow_is_a_plain_rejection() {
        for body in [
            &br#"{"error":{"message":"context window option is not supported"}}"#[..],
            br#"{"error":{"message":"Unknown parameter: 'context_management'."}}"#,
            br#"{"error":{"message":"context_window must be an integer"}}"#,
            br#"{"error":{"message":"This model's context window is 128k"}}"#,
            br#"{"error":{"message":"Invalid context length limit setting"}}"#,
        ] {
            let v = anthropic_error(StatusCode::BAD_REQUEST, body);
            assert_eq!(
                v["error"]["message"],
                "The upstream provider rejected the request.",
                "{}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn only_the_bounded_prefix_of_a_body_is_scanned() {
        let mut body = vec![b' '; MAX_ERROR_BODY];
        body.extend_from_slice(b"prompt is too long");
        let v = anthropic_error(StatusCode::BAD_REQUEST, &body);
        assert_ne!(v["error"]["message"], CONTEXT_MESSAGE);
    }
}
