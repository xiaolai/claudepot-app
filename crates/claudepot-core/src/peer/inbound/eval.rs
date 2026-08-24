//! Should the open grant be reverted? Pure, clock injected.
//!
//! Separated from the orchestrator so the precedence between "expired"
//! and "someone else changed it" is testable without a settings file, a
//! tick, or a wall clock.

use chrono::{DateTime, Utc};

use super::settings::ModeValue;
use super::{InboundGrant, InboundMode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// No grant on file. Nothing to do, and nothing to warn about.
    Idle,
    /// The window is open and has time left.
    Active { remaining_secs: i64 },
    /// The window closed. Restore `previous` and drop the record.
    Revert { previous: Option<InboundMode> },
    /// The setting no longer holds what the grant wrote, so someone
    /// changed it by hand. Drop the record **without** touching the
    /// setting: the user's own choice outranks our bookkeeping, and
    /// reverting here would silently undo a deliberate change.
    Superseded { observed: ModeValue },
}

/// `observed` is the current value of `crossSessionInbound`.
pub fn decide(grant: Option<&InboundGrant>, observed: &ModeValue, now: DateTime<Utc>) -> Decision {
    let Some(grant) = grant else {
        return Decision::Idle;
    };

    // Supersession is checked BEFORE expiry on purpose. A grant that
    // expired while the user was hand-editing the setting must still
    // not clobber what they chose — the deadline obliges us to stop
    // holding the door open, not to force it shut on their fingers.
    if observed.valid() != Some(grant.granted) {
        return Decision::Superseded {
            observed: observed.clone(),
        };
    }

    if grant.is_expired_at(now) {
        Decision::Revert {
            previous: grant.previous,
        }
    } else {
        Decision::Active {
            remaining_secs: grant.remaining_secs(now),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 22, h, 0, 0).unwrap()
    }

    fn grant(previous: Option<InboundMode>) -> InboundGrant {
        InboundGrant {
            granted: InboundMode::Accept,
            previous,
            granted_at: at(10),
            expires_at: at(12),
            reason: None,
        }
    }

    fn accepted() -> ModeValue {
        ModeValue::Valid(InboundMode::Accept)
    }

    #[test]
    fn no_grant_is_idle() {
        assert_eq!(decide(None, &ModeValue::Absent, at(11)), Decision::Idle);
    }

    #[test]
    fn a_live_grant_reports_time_remaining() {
        assert_eq!(
            decide(Some(&grant(None)), &accepted(), at(11)),
            Decision::Active {
                remaining_secs: 3600
            }
        );
    }

    #[test]
    fn an_expired_grant_reverts_to_absent() {
        assert_eq!(
            decide(Some(&grant(None)), &accepted(), at(12)),
            Decision::Revert { previous: None }
        );
    }

    #[test]
    fn an_expired_grant_restores_a_prior_value() {
        assert_eq!(
            decide(Some(&grant(Some(InboundMode::Refuse))), &accepted(), at(13)),
            Decision::Revert {
                previous: Some(InboundMode::Refuse)
            }
        );
    }

    #[test]
    fn a_hand_changed_setting_is_superseded_not_reverted() {
        let observed = ModeValue::Valid(InboundMode::Refuse);
        assert_eq!(
            decide(Some(&grant(None)), &observed, at(11)),
            Decision::Superseded { observed }
        );
    }

    #[test]
    fn supersession_beats_expiry() {
        // The window closed AND the user changed the setting. Reverting
        // would overwrite their choice with our stale record.
        let observed = ModeValue::Valid(InboundMode::Hold);
        assert_eq!(
            decide(Some(&grant(None)), &observed, at(20)),
            Decision::Superseded { observed }
        );
    }

    #[test]
    fn a_key_deleted_by_hand_is_superseded() {
        assert_eq!(
            decide(Some(&grant(None)), &ModeValue::Absent, at(11)),
            Decision::Superseded {
                observed: ModeValue::Absent
            }
        );
    }

    #[test]
    fn an_unrecognized_value_is_superseded_rather_than_overwritten() {
        // Something wrote a value CC will reject. That is a problem to
        // surface, not one to paper over by restoring our own idea of
        // the previous state.
        let observed = ModeValue::Unrecognized("yes".into());
        assert_eq!(
            decide(Some(&grant(None)), &observed, at(20)),
            Decision::Superseded { observed }
        );
    }
}
