//! One-shot boot check: is Claude Code about to delete saved
//! conversations on this machine?
//!
//! This is the whole reason Settings → Retention exists. CC's cleanup
//! runs on every launch, deletes transcripts older than
//! `cleanupPeriodDays`, computes an exact count of what it removed —
//! and discards it. Nothing in CC tells the user. A settings pane the
//! user must already suspect a problem to go looking for only half
//! closes that gap; this closes the other half.
//!
//! Per `rules/architecture.md` there is no logic here. The decision
//! ("should anything be said, and what") is
//! `claudepot_core::cc_retention::warning`, which is pure and tested.
//! This module only samples the clock, routes the result to the
//! notification log, and logs failures.
//!
//! Per `design.md`'s one-signal-per-surface rule this emits exactly one
//! entry, routed by the preferences of **whichever** of the two
//! mutually-exclusive retention categories applies:
//! `TranscriptsExpiring` when deletion is coming, and
//! `TranscriptCleanupSuppressed` when it has been switched off. Both are
//! P1. The split is what lets a user mute "conversations are expiring"
//! without also muting "your retention setting is broken" — so routing
//! must follow `category`, never a constant. No toast + banner + badge
//! spray.
//!
//! There is deliberately no "dismissed" flag; see
//! `cc_retention::RetentionWarning`'s docs for why gating on the
//! condition is both simpler and safer than persisting a suppression.

use claudepot_core::cc_retention::{
    cleanup_suppressed_warning, report, warning, CleanupSuppressedWarning, RetentionMode,
    RetentionWarning, DEFAULT_RISK_HORIZON_DAYS,
};
use claudepot_core::notification_log::NotificationKind;
use claudepot_core::notifications::{Category, Surface};
use tauri::{AppHandle, Manager};

use crate::commands::notification::NotificationLogState;
use crate::i18n::{tr, tr1, tr_n, tr_n_args};

/// Run the check off the setup thread. Scanning the transcript tree is
/// filesystem-bound (thousands of `stat` calls on a real corpus), so it
/// must never sit in front of window creation.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = run(&app) {
            tracing::warn!(error = %e, "retention_boot_check failed");
        }
    });
}

fn run(app: &AppHandle) -> Result<(), String> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let rep = report(now_ms, DEFAULT_RISK_HORIZON_DAYS);

    // Two mutually-exclusive conditions, one entry at most. `warning`
    // covers "deletion is coming"; `cleanup_suppressed_warning` covers
    // its mirror, "deletion has been switched off and you were not
    // told" — the state a legacy `cleanupPeriodDays: 0` now produces.
    // Core guarantees they cannot both fire; the `else if` is belt and
    // braces so a future core change degrades to one entry rather than
    // two contradictory ones.
    let Some((category, title, body)) = choose_entry(&rep) else {
        // Nothing scheduled for deletion, and cleanup is running
        // normally. Staying silent is the whole point of gating on the
        // condition rather than the setting.
        return Ok(());
    };

    let Some(log) = app.try_state::<NotificationLogState>() else {
        // Absent managed state is a wiring bug, not a normal condition
        // — say so rather than silently dropping the only proactive
        // data-loss warning in the product.
        return Err("NotificationLogState unavailable; retention warning dropped".to_string());
    };

    // Honour the user's per-category mute. `effective_os_surface`
    // cannot express this on its own: it returns an empty vec both for
    // "category muted" and for "enabled, but no OS banner wanted", so
    // muting has to be read from the pref directly or a muted category
    // would still append a bell entry every launch.
    let (enabled, os_surfaces) = {
        let prefs_state = app.state::<crate::preferences::PreferencesState>();
        let guard = match prefs_state.0.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                tracing::warn!(
                    "retention_boot_check: preferences mutex was poisoned by an earlier \
                     panic; recovering with under-lock data"
                );
                poisoned.into_inner()
            }
        };
        let enabled = guard.category_pref(category).enabled;
        // `wants_os: false` — the default surface is the bell, not an
        // OS banner: transcripts are days from deletion, not seconds,
        // so interrupting the desktop is unwarranted. A user who wants
        // the banner sets the category's os_override, which
        // `effective_os_surface` honours over this default. The
        // suppressed case is even less urgent — nothing is being lost
        // at all — so it takes the same default.
        let os = crate::preferences::effective_os_surface(&guard, category, false);
        (enabled, os)
    };
    if !enabled {
        tracing::debug!("retention_boot_check: category muted, no entry appended");
        return Ok(());
    }

    // Dispatch the OS banner when the user's override asked for one, so
    // `surfaces_requested` never records a request we knowingly
    // ignored.
    let mut delivered: Vec<Surface> = Vec::new();
    if os_surfaces.contains(&Surface::OsBanner) {
        use tauri_plugin_notification::NotificationExt;
        match app
            .notification()
            .builder()
            .title(&title)
            .body(&body)
            .show()
        {
            Ok(_) => delivered.push(Surface::OsBanner),
            Err(e) => tracing::warn!(error = %e, "retention_boot_check: OS notification failed"),
        }
    }

    log.log
        .append_routed(
            category,
            // Derived, never restated. Both retention categories are
            // P1 today; hardcoding that duplicated `Category::priority`
            // and would have made the log lie the moment either one was
            // re-prioritised.
            category.priority(),
            // `Notice`, not `Error`: nothing has failed. Something is
            // scheduled to happen that the user has not been told
            // about. `NotificationKind` has no Warning variant.
            NotificationKind::Notice,
            title,
            body,
            // Must match `NotificationTarget` in `src/lib/notify.ts` —
            // a discriminated union. The earlier flat
            // `{section, tab}` object had no `kind`, so clicking the
            // bell entry would have thrown instead of navigating.
            serde_json::json!({
                "kind": "app",
                "route": { "section": "settings", "sub": "retention" }
            }),
            os_surfaces,
            delivered,
        )
        .map_err(|e| format!("notification_log append failed: {e}"))?;

    tracing::info!(
        category = ?category,
        "retention_boot_check: warned about the machine's transcript retention state"
    );
    Ok(())
}

/// Localized body for the suppressed-cleanup entry. Mirrors
/// [`CleanupSuppressedWarning::message`] — which stays the CLI's English
/// surface — branch for branch; under `en` the two are byte-identical
/// (locked by the tests below).
fn suppressed_body(s: &CleanupSuppressedWarning) -> String {
    // An unread tree makes the count a floor. Saying "460" when we mean
    // "at least 460" is the same class of error as reporting a failed
    // scan as "nothing scheduled for deletion".
    let count = if s.scan_incomplete {
        tr1(
            "retention.countFloor",
            "num",
            &s.total_transcripts.to_string(),
        )
    } else {
        s.total_transcripts.to_string()
    };
    let key = if s.mode == RetentionMode::LegacyZero {
        "retention.suppressedLegacyZero"
    } else {
        "retention.suppressedInvalid"
    };
    tr1(key, "count", &count)
}

/// Pick the one entry this boot check may emit, if any.
///
/// Split out of [`run`] so the **routing** is testable without a Tauri
/// app handle. That matters more than it looks: the body text had
/// parity tests from the start, but the category did not, and the
/// category is what decides which mute preference applies. Sending the
/// suppressed warning under `TranscriptsExpiring` would look completely
/// correct in the bell and silently obey the wrong mute.
///
/// Returns `(category, title, body)`. The two conditions are mutually
/// exclusive in core; the `else if` keeps that true here even if that
/// ever changes, by preferring the "deletion is coming" entry.
fn choose_entry(
    rep: &claudepot_core::cc_retention::RetentionReport,
) -> Option<(Category, String, String)> {
    if let Some(w) = warning(rep) {
        Some((
            Category::TranscriptsExpiring,
            tr("retention.title"),
            warning_body(&w),
        ))
    } else {
        cleanup_suppressed_warning(rep).map(|s| {
            (
                Category::TranscriptCleanupSuppressed,
                tr("retention.suppressedTitle"),
                suppressed_body(&s),
            )
        })
    }
}

/// Localized banner body composed from [`RetentionWarning`]'s
/// structured fields. Mirrors `RetentionWarning::message()` — which
/// stays the CLI's English surface — branch for branch; under the `en`
/// locale the two are byte-identical (locked by the tests below).
fn warning_body(w: &RetentionWarning) -> String {
    if w.already_deletable == 0 && w.at_risk_within_horizon == 0 {
        // Only reachable via `scan_incomplete` — say what we don't
        // know rather than inventing a count.
        return tr("retention.scanIncomplete");
    }
    let head = if w.already_deletable > 0 {
        tr_n("retention.headDelete", w.already_deletable)
    } else {
        tr_n_args(
            "retention.headAtRisk",
            w.at_risk_within_horizon,
            &[("horizon", &w.horizon_days.to_string())],
        )
    };
    let why = if w.is_cc_default {
        tr1(
            "retention.whyDefault",
            "days",
            &w.effective_days.to_string(),
        )
    } else {
        String::new()
    };
    format!("{head}{why}{}", tr("retention.tail"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use claudepot_core::cc_retention::{RetentionReport, RetentionState, TranscriptRisk};

    fn warning_fixture(
        already_deletable: u64,
        at_risk_within_horizon: u64,
        is_cc_default: bool,
        scan_incomplete: bool,
    ) -> RetentionWarning {
        RetentionWarning {
            already_deletable,
            at_risk_within_horizon,
            effective_days: 30,
            is_cc_default,
            horizon_days: 7,
            scan_incomplete,
        }
    }

    /// The en composition must not drift from core's `message()` —
    /// that method remains the canonical English text (CLI surface),
    /// and this crate's tests run with the i18n global at its `En`
    /// default (no test calls `set_locale`).
    #[test]
    fn warning_body_matches_core_message_in_en() {
        let cases = [
            warning_fixture(0, 0, false, true),
            warning_fixture(1, 0, false, false),
            warning_fixture(3, 0, true, false),
            warning_fixture(0, 1, false, false),
            warning_fixture(0, 5, true, false),
        ];
        for w in &cases {
            assert_eq!(warning_body(w), w.message(), "fixture: {w:?}");
        }
    }

    fn suppressed_fixture(
        mode: RetentionMode,
        total_transcripts: u64,
        scan_incomplete: bool,
    ) -> CleanupSuppressedWarning {
        CleanupSuppressedWarning {
            mode,
            configured_days: if mode == RetentionMode::LegacyZero {
                Some(0)
            } else {
                None
            },
            total_transcripts,
            scan_incomplete,
        }
    }

    /// Build a report directly rather than through core's private
    /// `state_from_configured`; core's own tests already lock the
    /// setting-value → mode mapping, and widening its API for a test in
    /// another crate would be the wrong trade.
    fn report_for(mode: RetentionMode, total: u64, deletable: u64) -> RetentionReport {
        let suppressed = matches!(mode, RetentionMode::Invalid | RetentionMode::LegacyZero);
        RetentionReport {
            state: RetentionState {
                mode,
                configured_days: None,
                effective_days: 30,
                is_cc_default: mode == RetentionMode::CcDefault,
                cleanup_suppressed: suppressed,
            },
            risk: TranscriptRisk {
                total_transcripts: total,
                already_deletable: deletable,
                ..Default::default()
            },
            is_durable_archive: false,
            swept_elsewhere: Default::default(),
        }
    }

    /// The category is what selects the mute preference, so routing the
    /// suppressed state under `TranscriptsExpiring` would look right in
    /// the bell and obey the wrong preference. Body-text parity tests
    /// cannot see that; this can.
    #[test]
    fn each_retention_state_routes_to_its_own_category() {
        let expiring = report_for(RetentionMode::Explicit, 10, 4);
        let (cat, _, _) = choose_entry(&expiring).expect("deletion coming must warn");
        assert_eq!(cat, Category::TranscriptsExpiring);

        let legacy_zero = report_for(RetentionMode::LegacyZero, 10, 0);
        let (cat, _, body) = choose_entry(&legacy_zero).expect("suppressed must warn");
        assert_eq!(cat, Category::TranscriptCleanupSuppressed);
        assert!(body.contains("keeping every saved conversation"), "{body}");

        let invalid = report_for(RetentionMode::Invalid, 10, 0);
        let (cat, _, _) = choose_entry(&invalid).expect("invalid must warn");
        assert_eq!(cat, Category::TranscriptCleanupSuppressed);
    }

    /// A quiet machine emits nothing at all — the whole reason this is
    /// gated on the condition rather than on the setting.
    #[test]
    fn a_healthy_machine_emits_no_entry() {
        assert!(choose_entry(&report_for(RetentionMode::Explicit, 10, 0)).is_none());
        assert!(choose_entry(&report_for(RetentionMode::CcDefault, 0, 0)).is_none());
    }

    /// Same contract as `warning_body_matches_core_message_in_en`: core's
    /// `message()` is the canonical English (the CLI prints it), and the
    /// catalog composition must not drift from it.
    #[test]
    fn suppressed_body_matches_core_message_in_en() {
        let cases = [
            suppressed_fixture(RetentionMode::LegacyZero, 460, false),
            suppressed_fixture(RetentionMode::LegacyZero, 1, true),
            suppressed_fixture(RetentionMode::Invalid, 7, false),
            suppressed_fixture(RetentionMode::Invalid, 0, true),
        ];
        for s in &cases {
            assert_eq!(suppressed_body(s), s.message(), "fixture: {s:?}");
        }
    }

    /// The two bodies must never be confusable: one says deletion is
    /// coming, the other says it has stopped. A user reading the wrong
    /// one takes the wrong action on the only setting in Claude Code
    /// that destroys data.
    #[test]
    fn the_suppressed_body_never_claims_deletion_is_coming() {
        for s in [
            suppressed_fixture(RetentionMode::LegacyZero, 460, false),
            suppressed_fixture(RetentionMode::Invalid, 7, false),
        ] {
            let body = suppressed_body(&s);
            assert!(!body.contains("will be deleted"), "{body}");
            assert!(!body.contains("will delete"), "{body}");
        }
    }
}
