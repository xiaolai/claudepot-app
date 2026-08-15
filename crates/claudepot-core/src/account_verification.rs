//! Identity-verification types + persistence for the account store.
//!
//! Split out of `account.rs` to keep that file focused on the `Account`
//! struct and the core `AccountStore` CRUD operations. Re-exported from
//! `account.rs` so callers still see `claudepot_core::account::VerifyOutcome`.
//!
//! The data model: every account row has a `verify_status` that records
//! what the last `/api/oauth/profile` check said about the blob stored in
//! that slot. Six states:
//!
//! - `never` — reconciliation has not run yet (the post-migration default
//!   for pre-existing rows).
//! - `ok` — `/profile` returned the same email as the label.
//! - `drift` — `/profile` returned a *different* email. The slot is
//!   misfiled; the GUI paints a red banner, the CLI `account verify`
//!   exits non-zero.
//! - `rejected` — server returned 401 AND the refresh_token is also
//!   revoked. The user must re-login.
//! - `signed_out` — the blob is Claude Code's cleared-credentials
//!   sentinel: it parses, but both tokens are empty. Terminal like
//!   `rejected`, and kept separate from it because nothing was
//!   *refused* — no server call was even possible, so telling the user
//!   their login was rejected would point them at an account problem
//!   that does not exist.
//! - `network_error` — transient failure (transport / 5xx / rate-limit).
//!   The prior `verified_email` is preserved so a blip doesn't wipe the
//!   last-known-good identity.
//!
//! **`network_error` is the only non-terminal FAILURE in that list** —
//! `never` and `ok` are not failures at all, and are equally safe to
//! retry. Every consumer that branches on "should I retry / may I spend
//! quota here" must treat the terminal ones alike. A new terminal status that
//! only some of those filters know about is worse than no new status:
//! the account keeps burning refresh ticks and billable requests while
//! the UI says it is dead.

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
    /// The stored blob is Claude Code's cleared-credentials sentinel —
    /// it parses, but both tokens are empty (see
    /// [`CredentialBlob::is_signed_out`]). Terminal: only a re-login
    /// recovers it.
    ///
    /// Separate from [`Rejected`] because the *cause* differs and the
    /// cause is what the user acts on. `Rejected` means the server
    /// refused a real token — something happened to the account, and
    /// checking it is reasonable. `SignedOut` means no request was ever
    /// possible: Claude Code cleared its own local credentials and the
    /// account is untouched. Rendering one as the other sends the user
    /// to investigate a problem that isn't there.
    ///
    /// [`CredentialBlob::is_signed_out`]: crate::blob::CredentialBlob::is_signed_out
    /// [`Rejected`]: VerifyOutcome::Rejected
    SignedOut,
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
            VerifyOutcome::SignedOut => "signed_out",
            VerifyOutcome::NetworkError => "network_error",
        }
    }

    /// True when no retry can change this outcome — only a user action
    /// (re-login, or removing the account) can.
    ///
    /// The one predicate for "stop spending on this account". Every
    /// filter that used to spell `"drift" | "rejected"` inline gained a
    /// third case the day `SignedOut` landed, and an inline list has no
    /// way to announce that. Call this instead of re-deriving it.
    ///
    /// `Drift` is terminal in this sense too: the slot holds someone
    /// else's credentials, so refreshing entrenches a misfiling and
    /// billing against it attributes one person's usage to another.
    ///
    /// Delegates to [`status_is_terminal`] rather than re-matching the
    /// variants. A `matches!` here would have been a second encoding of
    /// the same set — the exact drift class this type exists to remove,
    /// reintroduced inside the type that removes it.
    ///
    /// [`status_is_terminal`]: VerifyOutcome::status_is_terminal
    pub fn is_terminal(&self) -> bool {
        Self::status_is_terminal(self.as_str())
    }

    /// Every `verify_status` string for which [`is_terminal`] holds.
    ///
    /// Public because two gates need the *set*, not just the predicate:
    /// `wake_service::UNVERIFIED_STATUSES` and
    /// `usage_cache::identity_gate` both refuse to spend on these. They
    /// used to carry hand-written copies, locked together by a test that
    /// compared one literal to an identical literal — which cannot
    /// observe the other module at all, so the two could have diverged
    /// with the test still green. They now read this.
    ///
    /// [`is_terminal`]: VerifyOutcome::is_terminal
    pub const TERMINAL_STATUSES: [&'static str; 3] = ["drift", "rejected", "signed_out"];

    /// [`is_terminal`] for a persisted `verify_status` string, which is
    /// the shape every consumer actually has in hand (DB column, DTO
    /// field, event payload).
    ///
    /// Unknown statuses are **not** terminal: a row written by a future
    /// version must degrade to "keep checking", never to "give up".
    ///
    /// [`is_terminal`]: VerifyOutcome::is_terminal
    pub fn status_is_terminal(status: &str) -> bool {
        Self::TERMINAL_STATUSES.contains(&status)
    }

    /// The action that actually clears a terminal status, as a clause
    /// that can be appended to an error sentence.
    ///
    /// Terminal-ness and *remedy* are different questions, and every
    /// gate that refuses an account has to answer the second one too.
    /// They were answering it wrongly in one voice: both the usage gate
    /// and the wake gate told the user to "run verify to reconcile" for
    /// every refusal, which is right for `drift` (verify re-reads the
    /// slot and can clear it) and useless for `rejected` / `signed_out`
    /// — verify will faithfully re-derive the same terminal answer
    /// forever, because only a re-login mints a token.
    ///
    /// Returns `None` for a non-terminal status: there is nothing for
    /// the user to do, so a caller that appends a remedy to a transient
    /// message is asking for action it does not need.
    pub fn status_remedy(status: &str) -> Option<&'static str> {
        match status {
            // The slot holds another account's credentials. Verify
            // re-reads it and reconciles the label, so it IS the fix.
            "drift" => Some("run `claudepot account verify` to reconcile"),
            // No token, or a token the server refuses. Nothing local
            // can mint one.
            "rejected" | "signed_out" => Some("log in again to restore it"),
            _ => None,
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
            // `verified_email` is preserved for all three. For the
            // transient case that is the documented blip semantics; for
            // the two terminal ones it is the last-known-good identity,
            // which is exactly what the UI needs to name the account the
            // user has to sign back in as.
            VerifyOutcome::Rejected | VerifyOutcome::SignedOut | VerifyOutcome::NetworkError => {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `as_str` feeds the DB column, the DTO, and the progress event.
    /// Wildcard-free by construction (the match in `as_str` is), but the
    /// literal spellings are a cross-boundary contract with TypeScript,
    /// so lock them here rather than trusting the enum alone.
    #[test]
    fn status_strings_are_stable() {
        assert_eq!(
            VerifyOutcome::Ok {
                email: "a@b.c".into()
            }
            .as_str(),
            "ok"
        );
        assert_eq!(
            VerifyOutcome::Drift {
                stored_email: "a@b.c".into(),
                actual_email: "d@e.f".into()
            }
            .as_str(),
            "drift"
        );
        assert_eq!(VerifyOutcome::Rejected.as_str(), "rejected");
        assert_eq!(VerifyOutcome::SignedOut.as_str(), "signed_out");
        assert_eq!(VerifyOutcome::NetworkError.as_str(), "network_error");
    }

    /// The two forms of the terminal predicate must agree. They are
    /// separate functions only because half the callers hold an enum
    /// and half hold a string; a disagreement between them would let an
    /// account be filtered out of one gate and through another.
    #[test]
    fn the_enum_and_string_terminal_predicates_agree() {
        for outcome in [
            VerifyOutcome::Ok {
                email: "a@b.c".into(),
            },
            VerifyOutcome::Drift {
                stored_email: "a@b.c".into(),
                actual_email: "d@e.f".into(),
            },
            VerifyOutcome::Rejected,
            VerifyOutcome::SignedOut,
            VerifyOutcome::NetworkError,
        ] {
            assert_eq!(
                outcome.is_terminal(),
                VerifyOutcome::status_is_terminal(outcome.as_str()),
                "{} disagrees between the enum and string predicates",
                outcome.as_str()
            );
        }
    }

    #[test]
    fn signed_out_is_terminal_and_network_error_is_not() {
        assert!(VerifyOutcome::SignedOut.is_terminal());
        assert!(!VerifyOutcome::NetworkError.is_terminal());
        // The regression this whole change is about: the sentinel used
        // to land on `network_error`, which is the one non-terminal
        // status — so the UI offered no action and the user waited.
        assert!(!VerifyOutcome::status_is_terminal("network_error"));
        assert!(VerifyOutcome::status_is_terminal("signed_out"));
    }

    /// A status this build has never heard of must read as "keep
    /// checking", not "give up". Failing the other way would let a
    /// downgrade silently strand an account.
    #[test]
    fn unknown_statuses_are_not_terminal() {
        assert!(!VerifyOutcome::status_is_terminal("never"));
        assert!(!VerifyOutcome::status_is_terminal("ok"));
        assert!(!VerifyOutcome::status_is_terminal("some_future_status"));
        assert!(!VerifyOutcome::status_is_terminal(""));
    }
}
