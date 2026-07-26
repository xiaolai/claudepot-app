//! Wake an account's rate-limit windows so `/api/oauth/usage` will
//! report their reset times.
//!
//! # Why this exists
//!
//! A rate-limit window is a property of a window that has *started*, not
//! of the account. Anthropic returns `resets_at: null` for a window with
//! no activity in it (see [`super::usage::UsageWindow::resets_at`]), and
//! the Accounts card renders that as `—`. Polling harder does not help:
//! `/api/oauth/usage` is a read, and reads do not start windows. The
//! snapshot writer already fetches every account every 5 minutes and
//! still sees `null`, because `null` is the honest answer.
//!
//! The only thing that starts a window is a billable request. So this
//! module sends the smallest one that exists.
//!
//! # What it costs
//!
//! Measured against a live Max account on 2026-07-25: 8 input + 1 output
//! token on Haiku, which the usage endpoint reported as **0.0%
//! utilization** — below its rounding threshold. For comparison,
//! `claude -p` would spend ~52k input tokens seeding Claude Code's
//! system prompt, so going direct to `/v1/messages` is roughly three
//! orders of magnitude cheaper. [`WAKE_MODEL`] and [`MAX_TOKENS`] are
//! the two knobs; neither should grow.
//!
//! # What the caller gets, and what it does NOT get
//!
//! Waking populates both windows, but they are not equally meaningful:
//!
//! - **7d** — the reset is a *pre-existing* schedule boundary. Measured
//!   at 5.43 days out when a naive "now + 7d" would have been 7.0, so
//!   the server is reporting the account's real weekly cycle, which was
//!   always there and merely unreported. This is genuine information.
//! - **5h** — the reset lands at wake time + 5h. That is a fact about
//!   *when Claudepot poked*, not about the user. Surfaces should not
//!   present it as though it were discovered.
//!
//! Callers that render the result are expected to keep that distinction
//! visible; see `AccountCard`'s wake affordance.
//!
//! # Why this is never automatic
//!
//! Spending a user's plan quota is a thing they ask for, not a thing a
//! background tick decides. A poller that woke every idle account would
//! spend forever on accounts deliberately being rested, and would keep
//! re-anchoring the 5h reset to its own schedule — manufacturing the
//! number it claims to report. There is deliberately no orchestrator
//! hook here: the CLI verb and the GUI menu item are the only callers.

use serde::{Deserialize, Serialize};

use super::{http_client, OAuthError};

/// Cheapest model available to a plan account. The window is shared
/// across models, so nothing is gained by using a larger one.
pub const WAKE_MODEL: &str = "claude-haiku-4-5";

/// One token out. The response body is discarded — we want the
/// side effect, not the text.
pub const MAX_TOKENS: u32 = 1;

/// Shortest prompt that still produces a valid request.
const WAKE_PROMPT: &str = "hi";

/// Rounded-up token cost, for disclosure *before* the user commits.
///
/// Measured at 8 input + 1 output against the live API. Owned here so
/// the CLI prompt and the GUI menu label quote the same number — this
/// is a cost disclosure, and two hand-written copies drift.
pub const ESTIMATED_TOKENS: u32 = 9;

#[derive(Serialize)]
struct WakeRequest {
    model: &'static str,
    max_tokens: u32,
    messages: [WakeMessage; 1],
}

#[derive(Serialize)]
struct WakeMessage {
    role: &'static str,
    content: &'static str,
}

/// Tokens actually spent, as reported by the API. Returned so callers
/// can show the user what the wake cost rather than asking them to
/// take "negligible" on faith.
///
/// Both fields default to 0 rather than failing the parse: the wake's
/// value is its *side effect*, so a response whose `usage` block is
/// missing or reshaped still succeeded. Reporting "0 tokens" is a
/// cosmetic loss; failing the call would tell the user nothing
/// happened when the window did in fact start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WakeCost {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

/// The slice of `/v1/messages` we care about. Typed rather than poked
/// at through `serde_json::Value` so the shape lives in one place and
/// the tests below exercise the same parser production uses.
#[derive(Debug, Deserialize)]
struct WakeResponse {
    #[serde(default)]
    usage: WakeCost,
}

/// Extract the token cost from a `/v1/messages` response body.
///
/// Separate from [`wake`] so tests can drive it with captured payloads.
/// Previously the test re-implemented this extraction inline, which
/// meant production parsing could drift while the test stayed green —
/// exactly the failure a parser test exists to prevent.
pub(crate) fn parse_wake_cost(body: &str) -> WakeCost {
    serde_json::from_str::<WakeResponse>(body)
        .map(|r| r.usage)
        .unwrap_or_default()
}

/// Send the minimal billable request, starting any rate-limit window
/// that had not yet started.
///
/// Returns the token cost. Does **not** re-read `/api/oauth/usage` — the
/// reset times take some seconds to propagate (observed: absent
/// immediately after the call, present by t+20s), so a caller that reads
/// back instantly would see the same `null` it started with and wrongly
/// conclude the wake failed. Let the existing 5-minute snapshot poll
/// pick it up, or re-read after a delay.
pub async fn wake(access_token: &str) -> Result<WakeCost, OAuthError> {
    let client = http_client()?;
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .bearer_auth(access_token)
        .header("anthropic-beta", super::beta_header::get_or_default())
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .json(&WakeRequest {
            model: WAKE_MODEL,
            max_tokens: MAX_TOKENS,
            messages: [WakeMessage {
                role: "user",
                content: WAKE_PROMPT,
            }],
        })
        .send()
        .await?;

    let status = resp.status();
    if status == 401 {
        return Err(OAuthError::AuthFailed(
            "access token rejected by /v1/messages".into(),
        ));
    }
    if status == 429 {
        // A wake that trips the rate limit has learned the answer it
        // wanted — the window is live and saturated — but the caller
        // still needs to know it spent nothing.
        return Err(OAuthError::RateLimited {
            retry_after_secs: super::retry_after_secs(resp.headers()),
        });
    }
    if !status.is_success() {
        // Consume the body without exposing it — it may echo request
        // content back.
        let _ = resp.text().await;
        // `ServerError`, not `AuthFailed`: a 5xx or a rejected model
        // name is the server having a bad minute, and telling the user
        // their credentials failed would send them to re-login for no
        // reason. See `OAuthError::ServerError`'s own doc comment.
        return Err(OAuthError::ServerError(format!(
            "wake request returned {status}"
        )));
    }

    Ok(parse_wake_cost(&resp.text().await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The request body is the whole cost surface. If someone grows the
    /// prompt or raises max_tokens, this test is the thing that argues
    /// back.
    #[test]
    fn the_request_body_stays_minimal() {
        let body = serde_json::to_value(WakeRequest {
            model: WAKE_MODEL,
            max_tokens: MAX_TOKENS,
            messages: [WakeMessage {
                role: "user",
                content: WAKE_PROMPT,
            }],
        })
        .unwrap();

        assert_eq!(body["max_tokens"], 1);
        assert_eq!(body["model"], "claude-haiku-4-5");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
        // Measured at 8 input tokens live. Anything longer is someone
        // spending the user's quota for no added signal.
        assert!(
            WAKE_PROMPT.len() <= 8,
            "wake prompt grew to {:?} — the window starts on any billable \
             request, so a longer prompt buys nothing",
            WAKE_PROMPT
        );
    }

    /// Verbatim body from the live `/v1/messages` call on 2026-07-25.
    const REAL_RESPONSE: &str = r#"{"model":"claude-haiku-4-5-20251001","id":"msg_011CdPaHzog7UypPuB4S497Z","type":"message","role":"assistant","content":[{"type":"text","text":"Hey"}],"stop_reason":"max_tokens","usage":{"input_tokens":8,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":1,"service_tier":"standard"}}"#;

    #[test]
    fn wake_cost_reads_the_usage_block() {
        // Drives the same `parse_wake_cost` production calls. The
        // earlier version of this test re-implemented the extraction,
        // so production could drift while the test stayed green.
        assert_eq!(
            parse_wake_cost(REAL_RESPONSE),
            WakeCost {
                input_tokens: 8,
                output_tokens: 1
            }
        );
    }

    /// The wake's value is its side effect. A response we cannot read
    /// the cost from still started the window, so these report zero
    /// rather than failing the call.
    #[test]
    fn an_unreadable_cost_reports_zero_rather_than_failing() {
        for body in [
            r#"{"id":"msg_1"}"#,                     // no usage block
            r#"{"usage":{}}"#,                       // empty usage
            r#"{"usage":{"output_tokens":1}}"#,      // partial usage
            "not json at all",                       // body isn't JSON
            r#"{"usage":{"input_tokens":"eight"}}"#, // wrong field type
        ] {
            let cost = parse_wake_cost(body);
            assert_eq!(
                cost.input_tokens + cost.output_tokens,
                if body.contains("\"output_tokens\":1") && body.contains("usage") {
                    1
                } else {
                    0
                },
                "unexpected cost for {body}"
            );
        }
    }

    #[test]
    fn the_disclosed_estimate_matches_the_measured_cost() {
        // ESTIMATED_TOKENS is quoted to the user *before* they commit.
        // If the real cost ever exceeds it, the disclosure is a lie.
        let measured = parse_wake_cost(REAL_RESPONSE);
        assert!(
            measured.input_tokens + measured.output_tokens <= u64::from(ESTIMATED_TOKENS),
            "measured {measured:?} exceeds the disclosed ESTIMATED_TOKENS={ESTIMATED_TOKENS}"
        );
    }
}
