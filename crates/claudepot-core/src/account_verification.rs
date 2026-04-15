//! Identity-verification types + persistence for the account store.
//!
//! Split out of `account.rs` to keep that file focused on the `Account`
//! struct and the core `AccountStore` CRUD operations. Re-exported from
//! `account.rs` so callers still see `claudepot_core::account::VerifyOutcome`.
//!
//! The data model: every account row has a `verify_status` that records
//! what the last `/api/oauth/profile` check said about the blob stored in
//! that slot. Five states:
//!
//! - `never` — reconciliation has not run yet (the post-migration default
//!   for pre-existing rows).
//! - `ok` — `/profile` returned the same email as the label.
//! - `drift` — `/profile` returned a *different* email. The slot is
//!   misfiled; the GUI paints a red banner, the CLI `account verify`
//!   exits non-zero.
//! - `rejected` — server returned 401 AND the refresh_token is also
//!   revoked. The user must re-login.
//! - `network_error` — transient failure (transport / 5xx / rate-limit).
//!   The prior `verified_email` is preserved so a blip doesn't wipe the
//!   last-known-good identity.

use crate::account::AccountStore;
use chrono::Utc;
use rusqlite::{params, Result as SqlResult};
use uuid::Uuid;

/// Result of an identity-verification pass against `/api/oauth/profile`.
/// Persisted to the account row via [`AccountStore::update_verification`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// Server confirmed the blob authenticates as the stored email.
    Ok { email: String },
    /// Server returned a profile email that doesn't match the stored email.
    /// The slot is misfiled — a refresh or switch could cross-contaminate.
    Drift {
        stored_email: String,
        actual_email: String,
    },
    /// Server rejected the token (401) AND refresh_token can't recover.
    /// Refresh can't fix it; re-login is required.
    Rejected,
    /// Transient failure (network, timeout, 5xx). Preserves any prior
    /// verified_email — a network blip must not wipe verification history.
    NetworkError,
}

impl VerifyOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            VerifyOutcome::Ok { .. } => "ok",
            VerifyOutcome::Drift { .. } => "drift",
            VerifyOutcome::Rejected => "rejected",
            VerifyOutcome::NetworkError => "network_error",
        }
    }
}

impl AccountStore {
    /// Persist a verification outcome on the account row. Called by
    /// `services::identity::verify_account_identity` after each `/profile`
    /// check. `VerifyOutcome::NetworkError` preserves `verified_email` so a
    /// transient blip doesn't wipe the last-known-good identity — only the
    /// status is updated.
    pub fn update_verification(&self, uuid: Uuid, outcome: &VerifyOutcome) -> SqlResult<()> {
        let status = outcome.as_str();
        let now = Utc::now().to_rfc3339();
        match outcome {
            VerifyOutcome::Ok { email } => {
                self.db().execute(
                    "UPDATE accounts SET verified_email = ?1, verified_at = ?2, \
                     verify_status = ?3 WHERE uuid = ?4",
                    params![email, now, status, uuid.to_string()],
                )?;
            }
            VerifyOutcome::Drift { actual_email, .. } => {
                self.db().execute(
                    "UPDATE accounts SET verified_email = ?1, verified_at = ?2, \
                     verify_status = ?3 WHERE uuid = ?4",
                    params![actual_email, now, status, uuid.to_string()],
                )?;
            }
            VerifyOutcome::Rejected | VerifyOutcome::NetworkError => {
                self.db().execute(
                    "UPDATE accounts SET verified_at = ?1, verify_status = ?2 \
                     WHERE uuid = ?3",
                    params![now, status, uuid.to_string()],
                )?;
            }
        }
        Ok(())
    }
}
