//! Time-boxed grants for Claude Code's cross-session inbound gate.
//!
//! `crossSessionInbound` decides what a session does with a peer
//! message: `accept` delivers it, `hold` parks it for the user's
//! approval, `refuse` rejects it. Holding is what an unattested sender
//! gets by default, which is correct for safety and useless for remote
//! control — a prompt sent from a phone parks behind a dialog on the
//! machine the user is not sitting at.
//!
//! Setting `accept` permanently is the obvious fix and the wrong one:
//! it leaves every session on the machine open to silent injection,
//! indefinitely. A grant closes by itself instead.
//!
//! ## Why the grant is machine-wide, and why that forces time-boxing
//!
//! Measured against CC 2.1.239, not assumed:
//!
//! - **`accept` only counts from user scope.** A project-scope value
//!   can *tighten* the gate but never loosen it — CC says so in as many
//!   words: "your own `accept` cannot override a repo tightening". A
//!   fresh session with `crossSessionInbound: accept` in
//!   `.claude/settings.local.json` still held.
//! - **Running sessions re-read it live.** A session started *before*
//!   the user-scope key was written delivered the next message, and
//!   went back to holding within seconds of the key being removed.
//!
//! The first fact is why this module writes [`SettingsLayer::User`] and
//! offers no project-scoped variant: one would silently do nothing.
//! It also means the blast radius cannot be narrowed *spatially* — so
//! it is narrowed *temporally*, and the deadline is the whole feature
//! rather than a convenience on top of it.
//!
//! The second fact is why that works at all: expiry genuinely closes
//! the door on sessions that are already running, instead of applying
//! only to ones started later.
//!
//! ## Relationship to `permission::grants`
//!
//! Same shape, deliberately: a dangerous state the user wants
//! temporarily and must not be trusted to switch off. `previous` is
//! recorded so revert restores the exact prior state including "the key
//! was absent", and the on-disk record is the only thing obliging the
//! revert — so [`store`] fails loud on corruption for the same reason
//! `permission::store` does.
//!
//! The difference is cardinality. A permission grant is per project and
//! several may be live; this key has one machine-wide value, so there
//! is at most one grant and the schema says so with an `Option` rather
//! than a `Vec`.

pub mod eval;
pub mod ops;
pub mod settings;
pub mod store;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use eval::{decide, Decision};
pub use ops::{open, revoke, status, tick};
pub use settings::{clear_mode, read_mode, write_mode, InboundSettingsError};
pub use store::{grant_path, load, save, GRANT_FILENAME};

/// Bumped on schema-breaking changes; the store moves an unrecognized
/// version aside.
pub const SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

/// CC's three inbound dispositions, over its own wire strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InboundMode {
    /// Deliver peer messages straight into the session's turn queue.
    Accept,
    /// Park them for the user to approve or deny. CC's effective
    /// default for an unattested sender.
    Hold,
    /// Reject them outright and tell the sender.
    Refuse,
}

impl InboundMode {
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Hold => "hold",
            Self::Refuse => "refuse",
        }
    }

    /// Parse CC's wire string. Unknown values are `None` rather than a
    /// guess: CC treats an invalid value as its own default, and
    /// pretending we know which one would misreport the gate's state.
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "accept" => Some(Self::Accept),
            "hold" => Some(Self::Hold),
            "refuse" => Some(Self::Refuse),
            _ => None,
        }
    }
}

/// On-disk shape of `~/.claudepot/peer-inbound-grant.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrantFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// At most one: the setting has a single machine-wide value.
    #[serde(default)]
    pub grant: Option<InboundGrant>,
}

impl Default for GrantFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            grant: None,
        }
    }
}

/// One open remote-control window.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InboundGrant {
    /// What Claudepot wrote. Recorded rather than assumed `Accept`, so
    /// a future narrower grant does not need a schema change.
    pub granted: InboundMode,
    /// What was there before, or `None` for "the key was absent".
    /// Revert restores exactly this — writing a default instead would
    /// pin a value the user never chose.
    pub previous: Option<InboundMode>,
    pub granted_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// Free text for the audit trail: who opened the window and why.
    #[serde(default)]
    pub reason: Option<String>,
}

impl InboundGrant {
    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
    }

    /// Whole seconds left, saturating at zero.
    pub fn remaining_secs(&self, now: DateTime<Utc>) -> i64 {
        (self.expires_at - now).num_seconds().max(0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GrantError {
    #[error("the grants file is not schema version {expected} (found {found})")]
    UnknownSchema { expected: u32, found: u32 },

    #[error("a grant is already open until {expires_at} — revoke it or let it expire")]
    AlreadyOpen { expires_at: DateTime<Utc> },

    #[error("grant duration must be at least 1 second")]
    ZeroDuration,

    #[error(
        "refusing a grant longer than {max_hours}h — the deadline is the \
         only thing containing this, since the setting is machine-wide"
    )]
    TooLong { max_hours: i64 },

    #[error(
        "crossSessionInbound currently holds {raw:?}, which Claude Code does \
         not recognise — fix it by hand first, so a grant has something it \
         can put back"
    )]
    UnrecognizedExistingValue { raw: String },

    #[error(transparent)]
    Settings(#[from] InboundSettingsError),

    #[error("cannot persist the grant record: {0}")]
    Store(String),
}

/// Upper bound on a single grant.
///
/// Not a safety property — the user can re-grant — but a forcing
/// function. An unbounded "grant" is just the permanent setting with
/// extra steps, which is the exact failure this module exists to
/// prevent.
pub const MAX_GRANT_HOURS: i64 = 12;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 22, h, 0, 0).unwrap()
    }

    fn grant() -> InboundGrant {
        InboundGrant {
            granted: InboundMode::Accept,
            previous: None,
            granted_at: at(10),
            expires_at: at(12),
            reason: Some("phone".into()),
        }
    }

    #[test]
    fn wire_strings_round_trip() {
        for m in [InboundMode::Accept, InboundMode::Hold, InboundMode::Refuse] {
            assert_eq!(InboundMode::from_wire(m.as_wire()), Some(m));
        }
    }

    #[test]
    fn an_unknown_wire_value_is_not_guessed() {
        assert_eq!(InboundMode::from_wire("Accept"), None);
        assert_eq!(InboundMode::from_wire(""), None);
        assert_eq!(InboundMode::from_wire("allow"), None);
    }

    #[test]
    fn mode_serializes_as_ccs_lowercase_wire_word() {
        assert_eq!(
            serde_json::to_string(&InboundMode::Accept).unwrap(),
            "\"accept\""
        );
    }

    #[test]
    fn expiry_is_inclusive_at_the_deadline() {
        let g = grant();
        assert!(!g.is_expired_at(at(11)));
        // At exactly the deadline the window is over — a grant that
        // survives its own expiry instant is a grant that can be
        // extended by a slow tick.
        assert!(g.is_expired_at(at(12)));
        assert!(g.is_expired_at(at(13)));
    }

    #[test]
    fn remaining_never_goes_negative() {
        let g = grant();
        assert_eq!(g.remaining_secs(at(11)), 3600);
        assert_eq!(g.remaining_secs(at(12)), 0);
        assert_eq!(g.remaining_secs(at(23)), 0);
    }

    #[test]
    fn absent_previous_round_trips_as_null() {
        let f = GrantFile {
            schema_version: SCHEMA_VERSION,
            grant: Some(grant()),
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: GrantFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, f);
        assert!(
            back.grant.unwrap().previous.is_none(),
            "\"the key was absent\" must survive a round trip, or revert \
             would write a value the user never chose"
        );
    }

    #[test]
    fn an_empty_file_defaults_to_no_grant() {
        let f: GrantFile = serde_json::from_str("{}").unwrap();
        assert_eq!(f.schema_version, SCHEMA_VERSION);
        assert!(f.grant.is_none());
    }
}
