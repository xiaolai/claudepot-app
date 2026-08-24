//! Opening, closing, and reconciling the remote-control window.
//!
//! The impure layer: everything here touches CC's settings file and the
//! grant record. The decisions it acts on are made by [`super::eval`],
//! which is pure.
//!
//! [`tick`] is the reconciler and is safe to call from anywhere, as
//! often as you like. That matters: the GUI orchestrator is the primary
//! caller, but a window left open by a Claudepot that is no longer
//! running must still close. Every CLI entry point that touches this
//! feature calls `tick` first, so `claudepot session send` alone is
//! enough to shut a window whose deadline passed while the app was
//! closed.

use chrono::{DateTime, Duration, Utc};

use super::eval::{decide, Decision};
use super::settings::{self, ModeValue};
use super::store;
use super::{GrantError, GrantFile, InboundGrant, InboundMode, MAX_GRANT_HOURS, SCHEMA_VERSION};

/// Open a window, returning the grant that was recorded.
///
/// Order is deliberate: reconcile first, then validate, then write the
/// setting, then persist the record. Writing the record *after* the
/// setting means a crash in between leaves the setting open with no
/// record — so the failure is loud (`tick` will report a value it has
/// no grant for) rather than silent.
pub fn open(
    duration: Duration,
    reason: Option<String>,
    now: DateTime<Utc>,
) -> Result<InboundGrant, GrantError> {
    // A stale expired grant must not block a new one.
    tick(now)?;

    if duration <= Duration::zero() {
        return Err(GrantError::ZeroDuration);
    }
    if duration > Duration::hours(MAX_GRANT_HOURS) {
        return Err(GrantError::TooLong {
            max_hours: MAX_GRANT_HOURS,
        });
    }

    let existing = load_grant()?;
    if let Some(g) = existing {
        if !g.is_expired_at(now) {
            return Err(GrantError::AlreadyOpen {
                expires_at: g.expires_at,
            });
        }
    }

    // Refuse rather than clobber a value we could not put back. This
    // preflight gives the caller the better error message; the
    // authoritative refusal is inside `write_mode`'s mutation closure,
    // because this read and that write take the lock separately.
    if let ModeValue::Unrecognized(raw) = settings::read_mode()? {
        return Err(GrantError::UnrecognizedExistingValue { raw });
    }

    let previous = match settings::write_mode(InboundMode::Accept) {
        Ok(p) => p,
        // Someone edited the file between the preflight and the write.
        Err(super::InboundSettingsError::UnrecognizedExisting { raw }) => {
            return Err(GrantError::UnrecognizedExistingValue { raw })
        }
        Err(e) => return Err(e.into()),
    };
    let grant = InboundGrant {
        granted: InboundMode::Accept,
        previous: previous.valid(),
        granted_at: now,
        expires_at: now + duration,
        reason,
    };

    // If the record cannot be written, put the setting back. The
    // original comment here claimed the opposite ordering made the
    // failure "loud" — it does not: `decide` returns `Idle` for an
    // `accept` it has no grant for, deliberately, so nothing would ever
    // close it. An unrecorded open window is the worst outcome this
    // function can produce, and rolling back is the only thing that
    // actually prevents it.
    if let Err(e) = persist(Some(grant.clone())) {
        if let Err(restore_err) = settings::restore(previous.valid()) {
            tracing::error!(
                error = %restore_err,
                "could not roll back crossSessionInbound after failing to \
                 record the grant — the window is OPEN with nothing minding \
                 it; close it by hand"
            );
        }
        return Err(e);
    }
    Ok(grant)
}

/// Close the window now, whatever its deadline said.
///
/// Takes no clock, and that is the point: "now, whatever its deadline
/// said" means the deadline is not consulted. It used to accept a `now`
/// it never read, silenced with `let _ = now;` — a parameter four
/// callers were computing and passing on a safety-sensitive API, which
/// reads as if the time mattered.
pub fn revoke() -> Result<Option<InboundGrant>, GrantError> {
    let Some(grant) = load_grant()? else {
        return Ok(None);
    };
    // Respect a hand-change here too: revoking should never write a
    // value the user did not ask for.
    let observed = settings::read_mode()?;
    if observed.valid() == Some(grant.granted) {
        settings::restore(grant.previous)?;
    }
    persist(None)?;
    Ok(Some(grant))
}

/// Reconcile the record against CC's settings. Idempotent.
pub fn tick(now: DateTime<Utc>) -> Result<Decision, GrantError> {
    let grant = load_grant()?;
    let observed = settings::read_mode()?;
    let decision = decide(grant.as_ref(), &observed, now);

    match &decision {
        Decision::Revert { previous } => {
            settings::restore(*previous)?;
            persist(None)?;
        }
        Decision::Superseded { .. } => {
            // Drop the record, touch nothing. The user's own edit stands.
            persist(None)?;
        }
        Decision::Idle | Decision::Active { .. } => {}
    }
    Ok(decision)
}

/// Current state without changing anything. For read-only surfaces.
pub fn status(now: DateTime<Utc>) -> Result<Decision, GrantError> {
    Ok(state(now)?.decision)
}

/// Everything a surface needs to describe the gate honestly.
///
/// `decision` alone is not enough. When the setting says `accept` but no
/// grant record exists, `decide` returns `Idle` — correct, because
/// Claudepot must not revert a value the user set themselves — and a UI
/// rendering only that would report "closed" while the door stands open.
/// `observed` is what makes the unmanaged case visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundState {
    pub decision: Decision,
    pub observed: ModeValue,
    /// The grant record was unreadable and has been reset. If the gate
    /// is also open, nothing is minding it and the user has to close it
    /// by hand — a surface that hides this is lying about the deadline.
    pub record_recovered: bool,
}

impl InboundState {
    /// Peer messages are being delivered without asking, whoever
    /// arranged it.
    pub fn is_open(&self) -> bool {
        self.observed.valid() == Some(InboundMode::Accept)
    }

    /// Open, but not by a grant Claudepot is holding a deadline on —
    /// so nothing will close it. Worth saying out loud in any UI.
    ///
    /// A recovered record counts even if a `decision` survived it: the
    /// deadline that decision came from is exactly what was lost.
    pub fn is_unmanaged_open(&self) -> bool {
        self.is_open()
            && (self.record_recovered || !matches!(self.decision, Decision::Active { .. }))
    }
}

pub fn state(now: DateTime<Utc>) -> Result<InboundState, GrantError> {
    let (grant, record_recovered) = load_grant_outcome()?;
    let observed = settings::read_mode()?;
    Ok(InboundState {
        decision: decide(grant.as_ref(), &observed, now),
        observed,
        record_recovered,
    })
}

fn load_grant() -> Result<Option<InboundGrant>, GrantError> {
    Ok(load_grant_outcome()?.0)
}

/// The grant plus whether the record had to be recovered from a corrupt
/// file.
///
/// The recovery marker used to be discarded here. That is the one place
/// it must not be: a recovered file means the deadline record is *gone*
/// while `crossSessionInbound` may still say `accept`, and because
/// nothing auto-reverts a value it has no grant for, the window would
/// stay open with no trace of why. Losing it silently is how a
/// three-minute grant becomes permanent.
fn load_grant_outcome() -> Result<(Option<InboundGrant>, bool), GrantError> {
    let loaded = store::load().map_err(|e| GrantError::Store(e.to_string()))?;
    Ok((loaded.value.grant, loaded.recovery.is_some()))
}

fn persist(grant: Option<InboundGrant>) -> Result<(), GrantError> {
    store::save(&GrantFile {
        schema_version: SCHEMA_VERSION,
        grant,
    })
    .map_err(|e| GrantError::Store(format!("{e:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Both the CC config dir and the Claudepot data dir have to be
    /// redirected: this feature writes one file in each.
    fn isolated() -> (TempDir, std::sync::MutexGuard<'static, ()>) {
        let lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let config = tmp.path().join("config-dir");
        let data = tmp.path().join("data-dir");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&data).unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", &config);
        std::env::set_var("CLAUDEPOT_DATA_DIR", &data);
        fs::write(config.join("settings.json"), r#"{"model":"opus"}"#).unwrap();
        (tmp, lock)
    }

    fn now() -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(2026, 8, 22, 10, 0, 0).unwrap()
    }

    #[test]
    fn open_sets_accept_and_records_the_window() {
        let (_t, _l) = isolated();
        let g = open(Duration::hours(2), Some("phone".into()), now()).unwrap();
        assert_eq!(g.granted, InboundMode::Accept);
        assert_eq!(g.previous, None);
        assert_eq!(g.expires_at, now() + Duration::hours(2));
        assert_eq!(
            settings::read_mode().unwrap(),
            ModeValue::Valid(InboundMode::Accept)
        );
    }

    #[test]
    fn a_second_open_while_one_is_live_is_refused() {
        let (_t, _l) = isolated();
        open(Duration::hours(2), None, now()).unwrap();
        assert!(matches!(
            open(Duration::hours(1), None, now()).unwrap_err(),
            GrantError::AlreadyOpen { .. }
        ));
    }

    #[test]
    fn zero_and_negative_durations_are_refused() {
        let (_t, _l) = isolated();
        assert!(matches!(
            open(Duration::zero(), None, now()).unwrap_err(),
            GrantError::ZeroDuration
        ));
        assert!(matches!(
            open(Duration::hours(-1), None, now()).unwrap_err(),
            GrantError::ZeroDuration
        ));
    }

    #[test]
    fn a_window_longer_than_the_cap_is_refused() {
        let (_t, _l) = isolated();
        assert!(matches!(
            open(Duration::hours(MAX_GRANT_HOURS + 1), None, now()).unwrap_err(),
            GrantError::TooLong { .. }
        ));
    }

    #[test]
    fn tick_before_expiry_leaves_the_window_open() {
        let (_t, _l) = isolated();
        open(Duration::hours(2), None, now()).unwrap();
        let d = tick(now() + Duration::hours(1)).unwrap();
        assert!(matches!(d, Decision::Active { .. }));
        assert_eq!(
            settings::read_mode().unwrap(),
            ModeValue::Valid(InboundMode::Accept)
        );
    }

    #[test]
    fn tick_after_expiry_closes_it_and_clears_the_record() {
        let (_t, _l) = isolated();
        open(Duration::hours(2), None, now()).unwrap();
        let d = tick(now() + Duration::hours(3)).unwrap();
        assert_eq!(d, Decision::Revert { previous: None });
        assert_eq!(settings::read_mode().unwrap(), ModeValue::Absent);
        assert!(load_grant().unwrap().is_none());
    }

    #[test]
    fn expiry_restores_a_prior_value_rather_than_deleting_the_key() {
        let (_t, _l) = isolated();
        fs::write(
            settings::user_settings_path(),
            r#"{"crossSessionInbound":"refuse"}"#,
        )
        .unwrap();
        open(Duration::hours(1), None, now()).unwrap();
        tick(now() + Duration::hours(2)).unwrap();
        assert_eq!(
            settings::read_mode().unwrap(),
            ModeValue::Valid(InboundMode::Refuse),
            "revert must restore what was there, not our idea of a default"
        );
    }

    #[test]
    fn tick_is_idempotent() {
        let (_t, _l) = isolated();
        open(Duration::hours(1), None, now()).unwrap();
        let later = now() + Duration::hours(2);
        tick(later).unwrap();
        assert_eq!(tick(later).unwrap(), Decision::Idle);
        assert_eq!(settings::read_mode().unwrap(), ModeValue::Absent);
    }

    #[test]
    fn a_hand_changed_setting_is_not_clobbered_by_expiry() {
        let (_t, _l) = isolated();
        open(Duration::hours(1), None, now()).unwrap();
        settings::write_mode(InboundMode::Refuse).unwrap();

        let d = tick(now() + Duration::hours(5)).unwrap();
        assert!(matches!(d, Decision::Superseded { .. }));
        assert_eq!(
            settings::read_mode().unwrap(),
            ModeValue::Valid(InboundMode::Refuse),
            "the user's own edit must survive our expiry"
        );
        assert!(
            load_grant().unwrap().is_none(),
            "the stale record is dropped"
        );
    }

    #[test]
    fn open_refuses_when_the_existing_value_is_unparseable() {
        let (_t, _l) = isolated();
        fs::write(
            settings::user_settings_path(),
            r#"{"crossSessionInbound":"yes"}"#,
        )
        .unwrap();
        // Overwriting would strand a value we could never restore.
        assert!(matches!(
            open(Duration::hours(1), None, now()).unwrap_err(),
            GrantError::UnrecognizedExistingValue { .. }
        ));
    }

    #[test]
    fn revoke_closes_early_and_reports_the_grant() {
        let (_t, _l) = isolated();
        open(Duration::hours(6), None, now()).unwrap();
        let g = revoke().unwrap();
        assert!(g.is_some());
        assert_eq!(settings::read_mode().unwrap(), ModeValue::Absent);
        assert!(load_grant().unwrap().is_none());
    }

    #[test]
    fn revoke_with_no_grant_is_a_no_op() {
        let (_t, _l) = isolated();
        assert!(revoke().unwrap().is_none());
    }

    #[test]
    fn an_expired_grant_does_not_block_a_new_one() {
        let (_t, _l) = isolated();
        open(Duration::hours(1), None, now()).unwrap();
        let later = now() + Duration::hours(4);
        // No explicit tick: `open` reconciles first.
        let g = open(Duration::hours(1), None, later).unwrap();
        assert_eq!(g.granted_at, later);
    }

    #[test]
    fn status_never_changes_anything() {
        let (_t, _l) = isolated();
        open(Duration::hours(1), None, now()).unwrap();
        let after = now() + Duration::hours(5);
        assert!(matches!(status(after).unwrap(), Decision::Revert { .. }));
        // Still open, because status only reports.
        assert_eq!(
            settings::read_mode().unwrap(),
            ModeValue::Valid(InboundMode::Accept)
        );
        assert!(load_grant().unwrap().is_some());
    }
}

#[cfg(test)]
mod state_tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn isolated() -> (TempDir, std::sync::MutexGuard<'static, ()>) {
        let lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let config = tmp.path().join("config-dir");
        let data = tmp.path().join("data-dir");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&data).unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", &config);
        std::env::set_var("CLAUDEPOT_DATA_DIR", &data);
        fs::write(config.join("settings.json"), r#"{"model":"opus"}"#).unwrap();
        (tmp, lock)
    }

    fn now() -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(2026, 8, 22, 10, 0, 0).unwrap()
    }

    #[test]
    fn a_closed_gate_is_neither_open_nor_unmanaged() {
        let (_t, _l) = isolated();
        let st = state(now()).unwrap();
        assert!(!st.is_open());
        assert!(!st.is_unmanaged_open());
    }

    #[test]
    fn a_granted_window_is_open_and_managed() {
        let (_t, _l) = isolated();
        open(Duration::hours(1), None, now()).unwrap();
        let st = state(now()).unwrap();
        assert!(st.is_open());
        assert!(!st.is_unmanaged_open(), "a live grant IS managed");
    }

    #[test]
    fn accept_set_by_hand_reads_as_open_but_unmanaged() {
        let (_t, _l) = isolated();
        // No grant record — the user did this themselves.
        settings::write_mode(InboundMode::Accept).unwrap();
        let st = state(now()).unwrap();
        assert_eq!(st.decision, Decision::Idle);
        assert!(
            st.is_open() && st.is_unmanaged_open(),
            "the door is open and nothing will close it — a UI that renders \
             only the decision would report this as closed"
        );
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn isolated() -> (TempDir, std::sync::MutexGuard<'static, ()>) {
        let lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let config = tmp.path().join("config-dir");
        let data = tmp.path().join("data-dir");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&data).unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", &config);
        std::env::set_var("CLAUDEPOT_DATA_DIR", &data);
        fs::write(config.join("settings.json"), r#"{"model":"opus"}"#).unwrap();
        (tmp, lock)
    }

    fn now() -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(2026, 8, 22, 10, 0, 0).unwrap()
    }

    /// The window stays open, the deadline record is destroyed, and
    /// nothing auto-reverts an `accept` it has no grant for. The only
    /// defence left is saying so — a state that reported "closed" here
    /// would leave the machine open indefinitely and silently.
    #[test]
    fn a_corrupt_record_over_an_open_gate_reads_as_unmanaged() {
        let (_t, _l) = isolated();
        open(Duration::hours(2), None, now()).unwrap();
        fs::write(store::grant_path(), "{ not json").unwrap();

        let st = state(now()).unwrap();
        assert!(st.record_recovered, "the corruption must be surfaced");
        assert!(st.is_open(), "the setting is still accept");
        assert!(
            st.is_unmanaged_open(),
            "the deadline is gone, so nothing will close this"
        );
    }

    #[test]
    fn a_healthy_open_window_is_not_flagged_as_recovered() {
        let (_t, _l) = isolated();
        open(Duration::hours(2), None, now()).unwrap();
        let st = state(now()).unwrap();
        assert!(!st.record_recovered);
        assert!(!st.is_unmanaged_open());
    }

    #[test]
    fn a_closed_gate_with_a_corrupt_record_is_not_open() {
        let (_t, _l) = isolated();
        fs::write(store::grant_path(), "{ not json").unwrap();
        let st = state(now()).unwrap();
        assert!(st.record_recovered);
        assert!(!st.is_open(), "no grant was ever applied to the setting");
        assert!(!st.is_unmanaged_open());
    }
}
