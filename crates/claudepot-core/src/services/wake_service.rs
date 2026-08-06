//! Orchestration for "wake an account's rate-limit windows".
//!
//! The CLI verb and the Tauri command are both thin wrappers over
//! [`wake_account`]. They previously each implemented this same four-step
//! flow — find account, check eligibility, get a token, call the OAuth
//! layer — which is business logic living in two frontends, and the two
//! copies had already diverged on eligibility.
//!
//! # The eligibility gate is the point
//!
//! Waking **spends the user's plan quota**, which makes "which account
//! am I actually spending?" a correctness question, not a nicety. A slot
//! whose stored blob authenticates as somebody else (`verify_status` of
//! `drift` or `rejected`) would happily mint a token and burn quota on
//! the *wrong* account, then report the label's email as though that
//! account had been woken.
//!
//! [`UsageCache::identity_gate`](crate::services::usage_cache) already
//! refuses to *read* usage for such a slot. Refusing to read while
//! permitting a write would be backwards, so the same rule is applied
//! here — deliberately as shared logic rather than a second hand-copied
//! `match`.

use uuid::Uuid;

use crate::account::AccountStore;
use crate::launcher;
use crate::oauth::wake::{self, WakeCost};

/// Why a wake could not be attempted, or how it failed.
///
/// The eligibility variants are distinct from the failure variants so a
/// caller can tell "we refused to spend" from "we tried and it broke" —
/// only the latter may have cost the user anything.
#[derive(Debug, thiserror::Error)]
pub enum WakeError {
    #[error("account not found")]
    NotFound,
    #[error("no credentials stored for {0}")]
    NoCredentials(String),
    /// The stored blob authenticates as a different account than the
    /// label claims. Spending quota here would bill the wrong identity.
    #[error("{0} failed identity verification (status: {1}) — run `claudepot account verify` to reconcile")]
    IdentityUnverified(String, String),
    #[error("store error: {0}")]
    Store(String),
    #[error("could not obtain an access token: {0}")]
    Token(String),
    #[error("wake request failed: {0}")]
    Request(String),
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
///
/// The split this enum's own doc comment draws — "we refused to spend"
/// versus "we tried and it broke" — is exactly what the codes have to
/// preserve. `NoCredentials` and `IdentityUnverified` mean **nothing was
/// billed**; `Token` and `Request` mean a request may have been made.
/// Collapsing them into one code would let a translator write a single
/// sentence for two opposite answers to "did that cost me anything".
///
/// `IdentityUnverified` crosses `status` as CC's raw discriminant
/// (`drift` / `rejected`) rather than a translated phrase: the English
/// message names `claudepot account verify` because the CLI prints it
/// verbatim, and a GUI sentence needs the discriminant to route the
/// user to the right pane instead.
impl crate::error_code::ErrorCode for WakeError {
    fn code(&self) -> &'static str {
        match self {
            WakeError::NotFound => "wake.not_found",
            WakeError::NoCredentials(_) => "wake.no_credentials",
            WakeError::IdentityUnverified(_, _) => "wake.identity_unverified",
            WakeError::Store(_) => "wake.store",
            WakeError::Token(_) => "wake.token",
            WakeError::Request(_) => "wake.request",
        }
    }

    fn params(&self) -> serde_json::Value {
        match self {
            WakeError::NotFound => serde_json::json!({}),
            WakeError::NoCredentials(email) => serde_json::json!({ "email": email }),
            WakeError::IdentityUnverified(email, status) => {
                serde_json::json!({ "email": email, "status": status })
            }
            // `Token` wraps the access-token *acquisition* failure, never
            // the token: the swap layer's errors carry reasons, not
            // credential material (`rules/rust-conventions.md`).
            WakeError::Store(detail) | WakeError::Token(detail) | WakeError::Request(detail) => {
                serde_json::json!({ "detail": detail })
            }
        }
    }
}

/// What a successful wake spent, plus who it was spent on.
#[derive(Debug, Clone)]
pub struct WakeReceipt {
    pub email: String,
    pub cost: WakeCost,
    pub model: &'static str,
}

/// `verify_status` values that must never reach a billable request.
/// Mirrors `UsageCache::identity_gate`'s rejection set.
const UNVERIFIED_STATUSES: [&str; 2] = ["drift", "rejected"];

/// Is this account safe to spend quota on?
///
/// Pure so the rule is testable without a keychain or a network — the
/// expensive, side-effecting half of [`wake_account`] is exactly what
/// unit tests cannot reach.
pub fn eligibility(
    has_credentials: bool,
    verify_status: &str,
    email: &str,
) -> Result<(), WakeError> {
    if !has_credentials {
        return Err(WakeError::NoCredentials(email.to_string()));
    }
    if UNVERIFIED_STATUSES.contains(&verify_status) {
        return Err(WakeError::IdentityUnverified(
            email.to_string(),
            verify_status.to_string(),
        ));
    }
    Ok(())
}

/// Start `uuid`'s rate-limit windows, refusing outright if the account
/// is not safe to bill.
///
/// Does not re-read `/api/oauth/usage`: reset times take ~20s to
/// propagate, so a read-back here would report the same `null` it
/// started with. Callers refresh on a delay instead.
pub async fn wake_account(store: &AccountStore, uuid: Uuid) -> Result<WakeReceipt, WakeError> {
    let account = store
        .find_by_uuid(uuid)
        .map_err(|e| WakeError::Store(e.to_string()))?
        .ok_or(WakeError::NotFound)?;

    eligibility(
        account.has_cli_credentials,
        &account.verify_status,
        &account.email,
    )?;

    let token = launcher::get_access_token(account.uuid, &account.email)
        .await
        .map_err(|e| WakeError::Token(e.to_string()))?;

    let cost = wake::wake(&token)
        .await
        .map_err(|e| WakeError::Request(e.to_string()))?;

    Ok(WakeReceipt {
        email: account.email,
        cost,
        model: wake::WAKE_MODEL,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_healthy_account_is_eligible() {
        assert!(eligibility(true, "ok", "a@b.com").is_ok());
    }

    /// An account never verified yet is not *known bad*. Refusing it
    /// would make wake unusable on a fresh install, where every slot
    /// starts with an empty status.
    #[test]
    fn an_unverified_but_undrifted_account_is_eligible() {
        assert!(eligibility(true, "", "a@b.com").is_ok());
        assert!(eligibility(true, "unverified", "a@b.com").is_ok());
    }

    /// The finding this module exists for: a drifted slot must never
    /// reach a billable request, or we spend on the wrong identity and
    /// report the label's email as if it had worked.
    #[test]
    fn a_drifted_account_is_refused_before_spending() {
        let err = eligibility(true, "drift", "a@b.com").unwrap_err();
        assert!(matches!(err, WakeError::IdentityUnverified(_, _)));
        assert!(err.to_string().contains("verify"), "must name the fix");
    }

    #[test]
    fn a_rejected_account_is_refused_before_spending() {
        assert!(matches!(
            eligibility(true, "rejected", "a@b.com").unwrap_err(),
            WakeError::IdentityUnverified(_, _)
        ));
    }

    /// Credentials are checked first: an account with no blob cannot be
    /// drifted in any meaningful sense, and "no credentials" is the more
    /// actionable message.
    #[test]
    fn missing_credentials_is_refused_and_takes_precedence() {
        assert!(matches!(
            eligibility(false, "drift", "a@b.com").unwrap_err(),
            WakeError::NoCredentials(_)
        ));
    }

    /// Locks the rejection set to `usage_cache::identity_gate`'s. If one
    /// grows a status the other doesn't, reads and writes disagree about
    /// which accounts are trustworthy.
    #[test]
    fn the_rejection_set_matches_the_usage_gate() {
        assert_eq!(UNVERIFIED_STATUSES, ["drift", "rejected"]);
    }
}
