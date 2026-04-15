//! `claudepot status` — ground-truth authentication check.
//!
//! Equivalent of running `claude auth status`, but also cross-checks
//! against Claudepot's stored `active_cli` pointer and exits with a
//! status code that scripts can branch on:
//!
//! | Exit | Meaning                                                  |
//! |------|----------------------------------------------------------|
//! | 0    | CC's authenticated email equals Claudepot's active_cli   |
//! | 2    | drift — CC is signed in as a different account          |
//! | 3    | couldn't check (no blob, corrupt blob, /profile failed)  |
//!
//! Use in CI:
//!
//! ```sh
//! claudepot status && echo "safe to proceed"
//! ```

use crate::AppContext;
use anyhow::Result;
use claudepot_core::account::AccountStore;
use claudepot_core::blob::CredentialBlob;
use claudepot_core::cli_backend;
use claudepot_core::cli_backend::swap::{DefaultProfileFetcher, ProfileFetcher};

/// Decision outcome for `claudepot status`. Separated from the
/// print/exit dispatch in `run()` so tests can assert on the exit-code
/// contract without running the real binary or calling
/// `std::process::exit`.
#[derive(Debug, PartialEq, Eq)]
pub enum StatusDecision {
    Match { cc_email: String },
    Drift { cc_email: String, active: String },
    Untracked { cc_email: String },
    NoBlob,
    CouldNotCheck(String),
}

impl StatusDecision {
    pub fn exit_code(&self) -> i32 {
        match self {
            StatusDecision::Match { .. } => 0,
            StatusDecision::Drift { .. } | StatusDecision::Untracked { .. } => 2,
            StatusDecision::NoBlob | StatusDecision::CouldNotCheck(_) => 3,
        }
    }
}

/// Pure-logic helper: given the raw CC email + Claudepot's active_cli
/// lookup result, decide the outcome. No I/O. This is what the tests
/// exercise directly.
pub fn classify(
    cc_email: Option<String>,
    active_cli_lookup: Result<Option<String>, String>,
) -> StatusDecision {
    let cc_email = match cc_email {
        Some(e) => e,
        None => return StatusDecision::NoBlob,
    };
    match active_cli_lookup {
        Ok(Some(active)) => {
            if active.eq_ignore_ascii_case(&cc_email) {
                StatusDecision::Match { cc_email }
            } else {
                StatusDecision::Drift { cc_email, active }
            }
        }
        Ok(None) => StatusDecision::Untracked { cc_email },
        Err(e) => StatusDecision::CouldNotCheck(e),
    }
}

/// Look up Claudepot's active_cli email. Returns `Ok(Some(email))`,
/// `Ok(None)` (no active_cli set), or `Err(reason)` to trigger exit 3.
fn active_cli_email(store: &AccountStore) -> Result<Option<String>, String> {
    match store.active_cli_uuid() {
        Ok(None) => Ok(None),
        Ok(Some(raw)) => match uuid::Uuid::parse_str(&raw) {
            Ok(u) => match store.find_by_uuid(u) {
                Ok(Some(a)) => Ok(Some(a.email)),
                Ok(None) => Ok(None),
                Err(e) => Err(format!("store find_by_uuid failed: {e}")),
            },
            Err(e) => Err(format!("active_cli uuid malformed: {e}")),
        },
        Err(e) => Err(format!("active_cli_uuid read failed: {e}")),
    }
}

pub async fn run(ctx: &AppContext) -> Result<()> {
    let platform = cli_backend::create_platform();
    let blob_str = match platform.read_default().await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return emit_and_exit(ctx, StatusDecision::NoBlob, None);
        }
        Err(e) => {
            return emit_and_exit(
                ctx,
                StatusDecision::CouldNotCheck(format!("couldn't read CC credentials: {e}")),
                None,
            );
        }
    };
    let blob = match CredentialBlob::from_json(&blob_str) {
        Ok(b) => b,
        Err(e) => {
            return emit_and_exit(
                ctx,
                StatusDecision::CouldNotCheck(format!("CC blob is not valid JSON: {e}")),
                None,
            );
        }
    };
    let fetcher = DefaultProfileFetcher;
    let cc_email = match fetcher
        .fetch_email(&blob.claude_ai_oauth.access_token)
        .await
    {
        Ok(e) => e,
        Err(e) => {
            return emit_and_exit(
                ctx,
                StatusDecision::CouldNotCheck(format!("/profile returned: {e}")),
                None,
            );
        }
    };

    let active_lookup = active_cli_email(&ctx.store);
    let decision = classify(Some(cc_email), active_lookup);
    emit_and_exit(ctx, decision, None)
}

/// Print the decision + exit with the correct code. `active_override`
/// is for tests that don't use a real ctx — production always passes None.
fn emit_and_exit(
    ctx: &AppContext,
    decision: StatusDecision,
    _active_override: Option<&str>,
) -> Result<()> {
    if ctx.json {
        emit_json(&decision);
    } else {
        emit_text(&decision);
    }
    let code = decision.exit_code();
    if code == 0 {
        Ok(())
    } else {
        std::process::exit(code);
    }
}

fn emit_text(decision: &StatusDecision) {
    match decision {
        StatusDecision::Match { cc_email } => {
            println!("CC:       {cc_email}");
            println!("Active:   {cc_email}");
            println!("Status:   MATCH");
        }
        StatusDecision::Drift { cc_email, active } => {
            println!("CC:       {cc_email}");
            println!("Active:   {active}");
            println!("Status:   DRIFT \u{2014} Claudepot's active_cli disagrees with CC");
        }
        StatusDecision::Untracked { cc_email } => {
            println!("CC:       {cc_email}");
            println!("Active:   (no active_cli set)");
            println!("Status:   UNTRACKED \u{2014} CC is signed in but Claudepot has no active_cli");
        }
        StatusDecision::NoBlob => {
            eprintln!("CC has no stored credentials");
        }
        StatusDecision::CouldNotCheck(msg) => {
            eprintln!("error: {msg}");
        }
    }
}

fn emit_json(decision: &StatusDecision) {
    let obj = match decision {
        StatusDecision::Match { cc_email } => serde_json::json!({
            "cc_email": cc_email,
            "claudepot_active_cli": cc_email,
            "match": true,
        }),
        StatusDecision::Drift { cc_email, active } => serde_json::json!({
            "cc_email": cc_email,
            "claudepot_active_cli": active,
            "match": false,
        }),
        StatusDecision::Untracked { cc_email } => serde_json::json!({
            "cc_email": cc_email,
            "claudepot_active_cli": serde_json::Value::Null,
            "match": false,
        }),
        StatusDecision::NoBlob => serde_json::json!({
            "cc_email": serde_json::Value::Null,
            "claudepot_active_cli": serde_json::Value::Null,
            "match": false,
            "error": "CC has no stored credentials",
        }),
        StatusDecision::CouldNotCheck(msg) => serde_json::json!({
            "cc_email": serde_json::Value::Null,
            "claudepot_active_cli": serde_json::Value::Null,
            "match": false,
            "error": msg,
        }),
    };
    println!("{obj}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_match_returns_exit_0() {
        let d = classify(Some("a@x.com".into()), Ok(Some("a@x.com".into())));
        assert_eq!(d, StatusDecision::Match { cc_email: "a@x.com".into() });
        assert_eq!(d.exit_code(), 0);
    }

    #[test]
    fn test_classify_drift_returns_exit_2() {
        let d = classify(Some("a@x.com".into()), Ok(Some("b@x.com".into())));
        assert_eq!(
            d,
            StatusDecision::Drift { cc_email: "a@x.com".into(), active: "b@x.com".into() }
        );
        assert_eq!(d.exit_code(), 2);
    }

    #[test]
    fn test_classify_untracked_returns_exit_2() {
        let d = classify(Some("a@x.com".into()), Ok(None));
        assert_eq!(d, StatusDecision::Untracked { cc_email: "a@x.com".into() });
        assert_eq!(d.exit_code(), 2);
    }

    #[test]
    fn test_classify_no_blob_returns_exit_3() {
        let d = classify(None, Ok(Some("a@x.com".into())));
        assert_eq!(d, StatusDecision::NoBlob);
        assert_eq!(d.exit_code(), 3);
    }

    #[test]
    fn test_classify_could_not_check_returns_exit_3() {
        let d = classify(
            Some("a@x.com".into()),
            Err("db corrupt".into()),
        );
        assert_eq!(d, StatusDecision::CouldNotCheck("db corrupt".into()));
        assert_eq!(d.exit_code(), 3);
    }

    #[test]
    fn test_classify_match_is_case_insensitive() {
        let d = classify(Some("Alice@Example.COM".into()), Ok(Some("alice@example.com".into())));
        assert!(matches!(d, StatusDecision::Match { .. }));
        assert_eq!(d.exit_code(), 0);
    }
}
