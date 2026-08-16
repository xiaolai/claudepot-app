//! Read + write CC's `cleanupPeriodDays` — the one Claude Code setting
//! that destroys user data, and the only one Claudepot must never let a
//! user meet by accident.
//!
//! # Why this gets its own module
//!
//! Every key in CC's settings schema whose description mentions
//! delete/remove/disable merely turns a feature off — `disableAllHooks`,
//! `prefersReducedMotion`, `alwaysThinkingEnabled`. Reversible, nothing
//! at stake. `cleanupPeriodDays` is different in four ways, and all four
//! are why a plain number input would be the wrong control:
//!
//! 1. **Sliding, not one-shot.** `getCutoffDate()` recomputes
//!    `now - cleanupPeriodDays` on *every* run, so the loss is
//!    continuous — roughly a day of corpus per day, forever.
//! 2. **Its "off-looking" value is not an off position — and no longer
//!    a value at all.** `0` never meant "no cleanup". Through CC 2.1.88
//!    it meant *disable session persistence entirely and delete existing
//!    transcripts at startup*; CC 2.1.233 rejects it outright, with a
//!    documented floor of 1. Either way `0` is not a stop on the
//!    duration scale, which is why [`RetentionMode`] gives it its own
//!    variant — see [`RetentionMode::LegacyZero`] for what a `0` still
//!    sitting on disk does today, which is the opposite of what whoever
//!    wrote it wanted.
//!
//!    **There is now no way to disable transcript persistence for an
//!    interactive session at all.** `--no-session-persistence` is
//!    `--print`-only and `persistSession: false` is an SDK option, so
//!    this module offers no "stop saving" verb; offering one would mean
//!    writing a value CC rejects and calling that success.
//! 3. **Silent.** `cleanupOldSessionFiles()` computes an exact count of
//!    the transcripts it just unlinked and its caller
//!    (`cleanupOldMessageFilesInBackground`) discards the return value.
//!    No log, no toast, no event. Nothing in CC ever tells the user.
//! 4. **Invisible on disk.** Cleanup unlinks the small, valuable
//!    top-level session transcripts and never walks `subagents/`, so the
//!    directory *grows* while history is destroyed. Disk usage can never
//!    reveal the loss — which is why [`TranscriptRisk`] reports
//!    `nested_immortal` alongside the at-risk count.
//!
//! # CC's resolution model
//!
//! Re-verified 2026-08-16 against the installed **2.1.233** binary,
//! which documents the key as "default: 30; minimum 1". The older
//! citation was `~/github/claude_code_src/src/utils/cleanup.ts:23`
//! (`DEFAULT_CLEANUP_PERIOD_DAYS = 30`) and `:27` — still correct on the
//! default, but that mirror is pinned at 2.1.88 and is exactly what
//! missed the floor change. Check the binary, not the mirror; see
//! `.claude/rules/cc-upstream-watch.md`.
//!
//! ```text
//! const days = settings.cleanupPeriodDays ?? 30
//! cutoff = now - days * 24h
//! // unlink any projects/<slug>/*.jsonl|*.cast whose mtime < cutoff
//! ```
//!
//! Unlike [`crate::auto_dream`], the absent case is a *known* constant
//! rather than a server flag, so this control is two-state (default vs
//! explicit) rather than three — but the default is still surfaced
//! distinctly, because "30 because nobody chose" and "30 because someone
//! chose 30" warrant different UI.
//!
//! # Naming
//!
//! Not `retention` — [`crate::retention`] already owns an unrelated
//! concept (Claudepot's own `activity_cards` / `metrics_tick` pruning
//! horizon). `cc_` prefix matches `cc_daemon` / `cc_doctor` / `cc_tips`
//! for CC-facing concerns.
//!
//! Global-only: transcript retention is a per-user setting, so this
//! writes `~/.claude/settings.json` and never a project layer.

use crate::paths::claude_config_dir;
use crate::settings_writer::{
    clear_bool_setting, mutate_settings, read_i64_setting, SettingValue, SettingsLayer,
    SettingsWriteError,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Setting key in CC's `settings.json`.
pub const CLEANUP_PERIOD_DAYS_KEY: &str = "cleanupPeriodDays";

/// CC's built-in default when the key is absent
/// (`utils/cleanup.ts:23 DEFAULT_CLEANUP_PERIOD_DAYS`).
pub const CC_DEFAULT_CLEANUP_PERIOD_DAYS: i64 = 30;

/// How far ahead [`TranscriptRisk`] looks when counting "about to be
/// deleted". Seven days is the shortest horizon that still gives a user
/// who opens Claudepot weekly a chance to act before the loss.
pub const DEFAULT_RISK_HORIZON_DAYS: i64 = 7;

const MS_PER_DAY: i64 = 24 * 60 * 60 * 1_000;

/// CC's floor for `cleanupPeriodDays`. Verified against the 2.1.233
/// binary, which documents the key as "default: 30; minimum 1" and
/// rejects `0` with its own message (see [`RetentionMode::LegacyZero`]).
///
/// Re-verify when the upstream watch (`dev-docs/cc-upstream-watch.md`)
/// reports movement on this key: it was `0` for the whole life of this
/// module before 2.1.233, and the change inverted a control.
pub const MIN_CLEANUP_PERIOD_DAYS: i64 = 1;

/// File extensions CC's `cleanupOldSessionFiles` unlinks by mtime.
/// Both are checked at `utils/cleanup.ts` in the `entry.isFile()` arm.
const CLEANED_EXTENSIONS: [&str; 2] = ["jsonl", "cast"];

/// What the current setting expresses. Deliberately **not** a bare
/// number: the UI must be unable to render `0` as an ordinary low value
/// on the same scale as `30`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RetentionMode {
    /// Key absent — CC's 30-day default silently applies. This is the
    /// state that cost this user five months of transcripts.
    CcDefault,
    /// An explicit positive day count.
    Explicit,
    /// `cleanupPeriodDays: 0` — **a value current CC rejects.**
    ///
    /// Its own variant rather than a fold into [`Self::Invalid`] because
    /// it is the one invalid value Claudepot itself used to write, on
    /// purpose, behind a type-to-confirm gate. Up to some release between
    /// 2.1.88 and 2.1.233 it meant "write no transcripts and delete the
    /// existing ones at startup". CC 2.1.233 rejects it outright:
    ///
    /// > `cleanupPeriodDays must be at least 1. … To disable transcript
    /// > writes entirely, remove this setting and use the
    /// > --no-session-persistence CLI flag or the SDK
    /// > persistSession:false option instead. (0 is rejected because it
    /// > previously silently disabled all transcript writes, which users
    /// > setting it to mean "never clean up" did not expect.)`
    ///
    /// So a `0` on disk today does the opposite of what whoever wrote it
    /// intended: transcripts **are** being written, and cleanup is
    /// suppressed because the key is present and fails validation. The
    /// user believes persistence is off while CC accumulates history
    /// forever. That inversion is why this keeps a distinct variant and
    /// its own repair copy — folding it into `Invalid` would render it as
    /// a typo to correct rather than a promise that was broken upstream.
    LegacyZero,
    /// A value CC's schema rejects: a number below 1, or anything that is
    /// not an integer at all (a string, a float, a bool, `null`). CC
    /// 2.1.233 documents the key as "default: 30; minimum 1", so any of
    /// these invalidates the whole settings file — which makes CC skip
    /// cleanup entirely. See [`RetentionState::cleanup_suppressed`]: the
    /// transcripts are safe, but only by accident, and "restore the
    /// default" would undo it.
    ///
    /// `0` is excluded here only because it gets [`Self::LegacyZero`];
    /// mechanically CC treats the two identically.
    ///
    /// The non-integer half of this used to read as `CcDefault`, because the
    /// reader collapsed "absent" and "present but wrong type" into one
    /// `None`. That told the user a 30-day timer was running on history that
    /// was in fact protected, and pointed them at the one button that would
    /// start the timer for real.
    Invalid,
}

/// Resolved retention state for one host's `~/.claude/settings.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetentionState {
    pub mode: RetentionMode,
    /// Raw value found in settings, if the key is present *and* an integer.
    /// `None` covers both "absent" and "present but not a number" — read
    /// [`Self::mode`] to tell those apart.
    pub configured_days: Option<i64>,
    /// The window CC would enforce if it were running cleanup at all.
    /// Meaningless when [`Self::cleanup_suppressed`] is set — check that
    /// first; for [`RetentionMode::Invalid`] this holds CC's nominal
    /// default purely so the field is never a surprising sentinel.
    pub effective_days: i64,
    /// True when nobody has chosen — the silent-default case.
    pub is_cc_default: bool,
    /// CC is currently not running cleanup at all.
    ///
    /// CC bails before touching any file when the settings file has
    /// validation errors *and* the raw settings explicitly contain
    /// `cleanupPeriodDays`. Any value below `1` triggers exactly that,
    /// and the key is by definition present. So does a non-integer
    /// value, for the same reason — the schema rejects `"thirty"`,
    /// `30.5`, and `true` alike.
    ///
    /// Re-verified against the 2.1.233 binary, which now says so in the
    /// user's own words rather than only in code:
    ///
    /// > `Skipping cleanup: settings have validation errors but
    /// > cleanupPeriodDays was explicitly set. Fix settings errors to
    /// > enable cleanup.`
    ///
    /// That is a meaningful upgrade on the 2.1.88 behaviour this module
    /// was written against, where the skip was entirely silent: CC now
    /// tells the user (via `/doctor`) that
    /// *"Transcript retention cleanup is paused until the settings
    /// errors above are fixed"*. The pane's advice is unchanged and now
    /// agrees with CC's: fix the value, never restore the default.
    ///
    /// Known limits, both in the safe direction:
    ///
    /// - CC suppresses cleanup when the settings file has *any*
    ///   validation error while this key is present, including an error
    ///   in an unrelated key. Claudepot does not model CC's whole
    ///   settings schema, so it detects only errors in this key. A file
    ///   invalid elsewhere reads here as "cleanup armed" when CC has in
    ///   fact suppressed it.
    /// - 2.1.233 adds two suppression causes Claudepot does not model at
    ///   all: a settings file that cannot be read or parsed, and
    ///   `--setting-sources` disabling the user-settings source while no
    ///   enabled source provides the key. Both also read here as
    ///   "cleanup armed".
    ///
    /// In every case we warn about deletion that is not happening, which
    /// is the direction to be wrong in — but it is not complete.
    ///
    /// So an invalid value accidentally *protects* transcripts. That
    /// changes the advice completely — "restore the default" would
    /// clear the error, re-arm cleanup, and delete everything past the
    /// 30-day line. The UI must say "fix the value", never "restore
    /// the default".
    pub cleanup_suppressed: bool,
}

/// Counts derived by walking the transcript tree the same way CC's
/// cleanup does.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscriptRisk {
    /// Top-level `projects/<slug>/*.jsonl|*.cast` files — everything CC
    /// is willing to unlink.
    pub total_transcripts: u64,
    /// Already older than the cutoff. CC deletes these on its next
    /// launch, with no further warning.
    pub already_deletable: u64,
    /// Will cross the cutoff within `horizon_days` (excludes
    /// `already_deletable`).
    pub at_risk_within_horizon: u64,
    /// Oldest surviving transcript, epoch ms.
    pub oldest_ms: Option<i64>,
    /// Files nested deeper than the project directory (`subagents/`,
    /// tool-result payloads). CC's cleanup never walks these, so they
    /// accumulate forever — this is why folder size can rise while
    /// history is being destroyed.
    pub nested_immortal: u64,
    pub horizon_days: i64,
    /// Some part of the tree could not be read (permissions, a
    /// transient I/O error, a `stat` that failed mid-walk).
    ///
    /// This exists because the counts above are otherwise
    /// indistinguishable between "nothing is at risk" and "we could not
    /// look". In a feature whose entire job is warning about silent
    /// data loss, a failed scan rendering as reassurance is the worst
    /// possible failure mode — so callers MUST NOT print "nothing is
    /// scheduled for deletion" while this is true.
    ///
    /// A missing projects directory is *not* incomplete: CC simply has
    /// no transcripts yet, which is a real and safe zero.
    pub scan_incomplete: bool,
}

/// Everything the Retention pane needs in one read: what is configured,
/// and what that costs on this machine right now.
///
/// Composed here rather than in the Tauri command so the command layer
/// stays free of logic (`rules/architecture.md`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetentionReport {
    pub state: RetentionState,
    pub risk: TranscriptRisk,
    /// Raising `cleanupPeriodDays` buys time; it does not make the
    /// corpus durable — the files still live in CC's cache directory
    /// under CC's rules. The pane states this so a large number cannot
    /// read as "solved". See the plan's P0 archive contract.
    pub is_durable_archive: bool,
}

/// Resolve settings and scan the live transcript tree. `now_ms` is
/// injected so the guillotine math stays testable.
pub fn report(now_ms: i64, horizon_days: i64) -> RetentionReport {
    let state = resolve_retention();
    let risk = scan_transcript_risk(&state, now_ms, horizon_days);
    RetentionReport {
        state,
        risk,
        // Always false today. Flipping this is P0's job, not a setting's.
        is_durable_archive: false,
    }
}

/// The one boot-time signal: transcripts on this machine are scheduled
/// for deletion and nothing else will say so.
///
/// # Why there is no "dismissed" flag
///
/// The plan asks that the warning "never fire again once dismissed with
/// retention raised". Persisting a dismissal would be the obvious
/// implementation and the wrong one — a dismissal flag silences the
/// warning while the data is still being deleted, which is precisely
/// the failure this whole feature exists to prevent.
///
/// Gating on the *condition* instead gets the requested behaviour for
/// free: raising retention drops `already_deletable` and
/// `at_risk_within_horizon` to zero, so [`warning`] returns `None` and
/// never fires again. Dismissing *without* raising retention lets it
/// return next boot — correct, because the transcripts really are still
/// on the timer. One entry per launch is not spam; it is the honest
/// report CC itself declines to make.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetentionWarning {
    pub already_deletable: u64,
    pub at_risk_within_horizon: u64,
    pub effective_days: i64,
    /// True when the user never chose this window — worth saying out
    /// loud, because "30 days" reads very differently once you know
    /// nobody picked it.
    pub is_cc_default: bool,
    pub horizon_days: i64,
    /// The transcript tree could not be fully read. The counts are a
    /// floor, not a total.
    pub scan_incomplete: bool,
}

/// Decide whether the boot check should surface anything. Pure.
///
/// Fires only when transcripts are actually at risk — never merely
/// because the setting is unset. A machine with no transcripts near the
/// cutoff has nothing to warn about, whatever its configuration.
pub fn warning(report: &RetentionReport) -> Option<RetentionWarning> {
    let r = &report.risk;
    // CC is not running cleanup at all, so nothing can be expiring
    // however little of the tree we managed to read. Checked before the
    // `scan_incomplete` clause below, which would otherwise fire a
    // "conversations are expiring" boot warning for a machine where
    // deletion is switched off — alarming the user about the one thing
    // that is not happening. The broken setting is still surfaced, but
    // in the pane, where it can be explained.
    if report.state.cleanup_suppressed {
        return None;
    }
    // An incomplete scan is not evidence of safety. Staying silent here
    // would mean a permissions failure reads exactly like "all clear",
    // in the one feature where that mistake costs the user their
    // history.
    if r.already_deletable == 0 && r.at_risk_within_horizon == 0 && !r.scan_incomplete {
        return None;
    }
    Some(RetentionWarning {
        already_deletable: r.already_deletable,
        at_risk_within_horizon: r.at_risk_within_horizon,
        effective_days: report.state.effective_days,
        is_cc_default: report.state.is_cc_default,
        horizon_days: r.horizon_days,
        scan_incomplete: r.scan_incomplete,
    })
}

impl RetentionWarning {
    /// One-line body for the bell entry. Leads with the count, names
    /// the silence, and states the fix — the same shape as the pane.
    pub fn message(&self) -> String {
        if self.already_deletable == 0 && self.at_risk_within_horizon == 0 {
            // Only reachable via `scan_incomplete` — say what we don't
            // know rather than inventing a count.
            return "Claudepot could not read part of Claude Code's transcript folder, \
                    so it cannot tell you what is scheduled for deletion. \
                    Settings → Retention."
                .to_string();
        }
        let head = if self.already_deletable > 0 {
            format!(
                "Claude Code will delete {} saved conversation{} on its next launch",
                self.already_deletable,
                if self.already_deletable == 1 { "" } else { "s" }
            )
        } else {
            format!(
                "{} saved conversation{} will be deleted within {} days",
                self.at_risk_within_horizon,
                if self.at_risk_within_horizon == 1 {
                    ""
                } else {
                    "s"
                },
                self.horizon_days
            )
        };
        let why = if self.is_cc_default {
            format!(
                " — its {}-day default, which nobody chose",
                self.effective_days
            )
        } else {
            String::new()
        };
        format!("{head}{why}. Settings → Retention.")
    }
}

/// The mirror of [`RetentionWarning`]: cleanup is switched **off** and
/// the user has not been told.
///
/// [`warning`] deliberately returns `None` for every suppressed state,
/// because announcing "conversations are expiring" for a machine where
/// deletion is disabled alarms the user about the one thing that is not
/// happening. That leaves a real gap it was never meant to cover — a
/// suppressed setting is only discoverable by opening the pane, and the
/// user with the strongest reason to look is precisely the one who
/// believes the matter is settled.
///
/// Two states reach here and they are not equally surprising:
///
/// - [`RetentionMode::LegacyZero`] — the user chose "stop saving" in an
///   older Claudepot and got the opposite. Their belief about what is on
///   disk is actively wrong, which is a privacy question, not just a
///   settings one.
/// - [`RetentionMode::Invalid`] — someone typed a value CC rejects. The
///   transcripts are protected by accident, and the protection
///   evaporates the moment anyone "tidies up" the settings file.
///
/// Both are worth one line at boot; the copy differs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanupSuppressedWarning {
    /// Which suppressed state. Always `LegacyZero` or `Invalid` —
    /// [`cleanup_suppressed_warning`] returns `None` for anything else.
    pub mode: RetentionMode,
    /// The raw value on disk when it was an integer at all — `Some(0)`
    /// for a legacy zero, `None` when the value was a string, float,
    /// bool or null.
    pub configured_days: Option<i64>,
    /// How much history has accumulated while nothing was cleaning it.
    /// The point of carrying it: for a legacy zero this is the number
    /// the user believes is zero.
    pub total_transcripts: u64,
    /// The transcript tree could not be fully read, so
    /// `total_transcripts` is a floor. Never render it as a total while
    /// this is set.
    pub scan_incomplete: bool,
}

impl CleanupSuppressedWarning {
    /// One-line body for the bell entry, and the CLI's English surface.
    /// The GUI mirrors this branch for branch through the catalog.
    pub fn message(&self) -> String {
        let count = if self.scan_incomplete {
            format!("at least {}", self.total_transcripts)
        } else {
            self.total_transcripts.to_string()
        };
        match self.mode {
            RetentionMode::LegacyZero => format!(
                "Claude Code is keeping every saved conversation ({count} on this \
                 machine). `cleanupPeriodDays: 0` once meant \"stop saving\"; Claude \
                 Code now rejects it, so transcripts are being written and cleanup is \
                 switched off. Settings → Retention."
            ),
            _ => format!(
                "Claude Code has stopped deleting saved conversations ({count} on this \
                 machine) because `cleanupPeriodDays` holds a value it rejects. Nothing \
                 is being lost, but the protection is accidental. Settings → Retention."
            ),
        }
    }
}

/// Decide whether the boot check should report a *suppressed* cleanup.
/// Pure, and mutually exclusive with [`warning`] by construction: that
/// one returns `None` whenever `cleanup_suppressed` is set, and this one
/// returns `None` whenever it is not.
///
/// Gated on the condition, with no dismissal flag, for exactly the
/// reason given on [`RetentionWarning`]: fixing the setting silences it,
/// and dismissing without fixing should not.
pub fn cleanup_suppressed_warning(report: &RetentionReport) -> Option<CleanupSuppressedWarning> {
    if !report.state.cleanup_suppressed {
        return None;
    }
    Some(CleanupSuppressedWarning {
        mode: report.state.mode,
        configured_days: report.state.configured_days,
        total_transcripts: report.risk.total_transcripts,
        scan_incomplete: report.risk.scan_incomplete,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum RetentionError {
    #[error("write CC settings")]
    Write(#[from] SettingsWriteError),
    #[error(
        "cleanupPeriodDays must be at least 1; got {0}. Claude Code rejects \
         anything lower, and a rejected value does not disable persistence — \
         it makes CC skip cleanup entirely."
    )]
    NonPositive(i64),
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
///
/// Two things here are load-bearing beyond the usual contract, both
/// from this module's header:
///
/// - **`NonPositive` keeps its own code.** It is the guard that stops
///   `0` from riding the duration scale. CC now rejects `0` itself, so
///   this is no longer the last line of defence it once was — but it is
///   still the only place that can explain *why* the write was refused,
///   and the stakes are not those of an ordinary out-of-range number:
///   a `0` that reached the file suppresses cleanup and leaves the user
///   believing persistence is off while CC keeps writing. Folding it
///   into a generic "invalid value" code would let a translator write
///   one sentence for two very different situations.
/// - **Nothing here is the `cleanup_suppressed` case.** A value CC's
///   schema rejects *protects* transcripts and is reported as
///   `RetentionMode::Invalid` / `RetentionMode::LegacyZero` state, never
///   as an error — which is why no code in this impl may ever be given
///   "restore the default" remediation copy. Restoring re-arms deletion.
impl crate::error_code::ErrorCode for RetentionError {
    fn code(&self) -> &'static str {
        match self {
            RetentionError::Write(_) => "cc_retention.write",
            RetentionError::NonPositive(_) => "cc_retention.non_positive",
        }
    }

    fn params(&self) -> serde_json::Value {
        match self {
            // `Display` here is the bare label "write CC settings" —
            // the source is deliberately not folded into it, so a GUI
            // with no `--verbose` would otherwise have nothing to show.
            // `detail` is what makes the failure diagnosable; the
            // English `message` is unchanged either way.
            RetentionError::Write(e) => serde_json::json!({ "detail": e.to_string() }),
            // The rejected day count. The CLI prints the English
            // sentence verbatim; a GUI sentence renders from the
            // catalog and needs the number to say what the user typed.
            RetentionError::NonPositive(days) => serde_json::json!({ "days": days }),
        }
    }
}

fn user_settings_path() -> PathBuf {
    claude_config_dir().join("settings.json")
}

fn projects_dir() -> PathBuf {
    claude_config_dir().join("projects")
}

/// Resolve retention from `~/.claude/settings.json`. Pure read.
pub fn resolve_retention() -> RetentionState {
    // A read failure is not "no key". An unreadable or unparseable settings
    // file tells us nothing about what CC will do, and the conservative
    // reading is the one that does not promise the user a deletion window we
    // could not verify: treat it as invalid, which is also what CC does with
    // a file it cannot validate.
    let configured = read_i64_setting(&user_settings_path(), CLEANUP_PERIOD_DAYS_KEY)
        .unwrap_or(SettingValue::Invalid);
    state_from_configured(configured)
}

/// Split out so the resolution table is testable without a filesystem.
fn state_from_configured(configured: SettingValue<i64>) -> RetentionState {
    let (mode, effective_days) = match configured {
        SettingValue::Absent => (RetentionMode::CcDefault, CC_DEFAULT_CLEANUP_PERIOD_DAYS),
        // Present but not an integer — CC's schema rejects it and, with the
        // key present, skips cleanup entirely.
        SettingValue::Invalid => (RetentionMode::Invalid, CC_DEFAULT_CLEANUP_PERIOD_DAYS),
        // Carries CC's nominal default rather than `0` for the same
        // reason `Invalid` does: cleanup is suppressed, so this field is
        // meaningless here, and a `0` sitting in it invites a caller to
        // render "deletes everything after 0 days" — the exact claim
        // that is no longer true.
        SettingValue::Present(0) => (RetentionMode::LegacyZero, CC_DEFAULT_CLEANUP_PERIOD_DAYS),
        SettingValue::Present(d) if d < 1 => {
            (RetentionMode::Invalid, CC_DEFAULT_CLEANUP_PERIOD_DAYS)
        }
        SettingValue::Present(d) => (RetentionMode::Explicit, d),
    };
    RetentionState {
        mode,
        configured_days: configured.ok(),
        effective_days,
        is_cc_default: mode == RetentionMode::CcDefault,
        // `LegacyZero` suppresses cleanup exactly as `Invalid` does: CC
        // requires >= 1, so the key is present and fails validation,
        // which is the documented skip condition.
        cleanup_suppressed: matches!(mode, RetentionMode::Invalid | RetentionMode::LegacyZero),
    }
}

/// Count transcripts against the cutoff CC will apply, walking exactly
/// what `cleanupOldSessionFiles` walks: files sitting *directly* in each
/// `projects/<slug>/` directory. Anything deeper is counted separately
/// as `nested_immortal` — cleanup never reaches it.
///
/// `now_ms` is injected so the guillotine math is testable.
pub fn scan_transcript_risk(
    state: &RetentionState,
    now_ms: i64,
    horizon_days: i64,
) -> TranscriptRisk {
    scan_transcript_risk_in(&projects_dir(), state, now_ms, horizon_days)
}

/// [`scan_transcript_risk`] against an explicit root, so tests (and a
/// future archive importer) can point it at a directory that is not the
/// live `~/.claude/projects`.
pub fn scan_transcript_risk_in(
    root: &Path,
    state: &RetentionState,
    now_ms: i64,
    horizon_days: i64,
) -> TranscriptRisk {
    let mut risk = TranscriptRisk {
        horizon_days,
        ..Default::default()
    };
    // Every step is saturating: `now_ms` and `effective_days` both come
    // from outside (a clock and a user-editable settings file), so an
    // absurd value must clamp rather than overflow. `i64` subtraction
    // panics in debug and wraps in release, and a wrapped cutoff would
    // invert the risk classification — reporting "safe" for transcripts
    // that are about to be deleted.
    // When CC is skipping cleanup entirely (invalid settings — see
    // `RetentionState::cleanup_suppressed`), no file is at risk however
    // old it is. Pushing the cutoff to the floor keeps the counting
    // loop uniform instead of branching around it.
    let (cutoff_ms, horizon_cutoff_ms) = if state.cleanup_suppressed {
        (i64::MIN, i64::MIN)
    } else {
        let cutoff = now_ms.saturating_sub(state.effective_days.saturating_mul(MS_PER_DAY));
        (
            cutoff,
            cutoff.saturating_add(horizon_days.saturating_mul(MS_PER_DAY)),
        )
    };

    let projects = match std::fs::read_dir(root) {
        Ok(p) => p,
        // A missing projects dir is a real zero (CC has written no
        // transcripts). Anything else means we could not look, which
        // must never be rendered as reassurance.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return risk,
        Err(_) => {
            risk.scan_incomplete = true;
            return risk;
        }
    };
    for project in projects {
        let Ok(project) = project else {
            risk.scan_incomplete = true;
            continue;
        };
        match project.file_type() {
            Ok(t) if t.is_dir() => {}
            Ok(_) => continue,
            Err(_) => {
                risk.scan_incomplete = true;
                continue;
            }
        }
        let entries = match std::fs::read_dir(project.path()) {
            Ok(e) => e,
            Err(_) => {
                risk.scan_incomplete = true;
                continue;
            }
        };
        for entry in entries {
            let Ok(entry) = entry else {
                risk.scan_incomplete = true;
                continue;
            };
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => {
                    risk.scan_incomplete = true;
                    continue;
                }
            };
            if ft.is_dir() {
                let (n, ok) = count_nested(&entry.path());
                risk.nested_immortal += n;
                if !ok {
                    risk.scan_incomplete = true;
                }
                continue;
            }
            if !is_cleaned_file(&entry.path()) {
                continue;
            }
            risk.total_transcripts += 1;
            let Some(mtime_ms) = mtime_ms(&entry.path()) else {
                // We know a transcript is here but not how old it is,
                // so it cannot be classified either way.
                risk.scan_incomplete = true;
                continue;
            };
            risk.oldest_ms = Some(match risk.oldest_ms {
                Some(prev) if prev <= mtime_ms => prev,
                _ => mtime_ms,
            });
            if mtime_ms < cutoff_ms {
                risk.already_deletable += 1;
            } else if mtime_ms < horizon_cutoff_ms {
                risk.at_risk_within_horizon += 1;
            }
        }
    }
    risk
}

/// Recursively count `.jsonl` / `.cast` files below a session directory.
/// These are the ones cleanup never unlinks.
///
/// Returns `(count, complete)` — `complete == false` when any part of
/// the subtree could not be read, so the caller can mark the whole scan
/// incomplete rather than under-reporting silently.
fn count_nested(dir: &Path) -> (u64, bool) {
    let mut n = 0;
    let mut complete = true;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, false);
    };
    for entry in entries {
        let Ok(entry) = entry else {
            complete = false;
            continue;
        };
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => {
                let (sub, ok) = count_nested(&entry.path());
                n += sub;
                complete &= ok;
            }
            Ok(_) => {
                if is_cleaned_file(&entry.path()) {
                    n += 1;
                }
            }
            Err(_) => complete = false,
        }
    }
    (n, complete)
}

fn is_cleaned_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| CLEANED_EXTENSIONS.contains(&e))
        .unwrap_or(false)
}

fn mtime_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let dur = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(dur.as_millis()).ok()
}

/// Write an explicit retention window to `~/.claude/settings.json`.
///
/// Refuses anything below `1`, which is CC's own floor — the 2.1.233
/// binary states the key is "default: 30; minimum 1" and rejects `0`
/// with a dedicated message. A rejected value does not disable
/// anything; it invalidates the settings file and makes CC skip
/// cleanup, so writing one is never a way to express intent.
///
/// **There is deliberately no "disable persistence" sibling to this
/// function.** Current CC offers no settings-file route to disabling
/// transcript writes, and the two routes it does offer are out of
/// Claudepot's reach for the sessions this pane is about:
/// `--no-session-persistence` is rejected outside `--print` mode
/// (`Error: --no-session-persistence can only be used with --print
/// mode.`), and `persistSession: false` is an SDK option. An
/// interactive Claude Code session cannot have persistence turned off
/// at all, so a control offering to do it would be a lie whatever it
/// wrote.
pub fn set_retention_days(days: i64) -> Result<(), RetentionError> {
    // For an integer these are the same set, so the variant keeps its
    // name (and its error code, which the i18n catalog is keyed on);
    // the constant is what states CC's actual rule.
    if days < MIN_CLEANUP_PERIOD_DAYS {
        return Err(RetentionError::NonPositive(days));
    }
    mutate_settings(SettingsLayer::User, Path::new(""), |obj| {
        obj.insert(
            CLEANUP_PERIOD_DAYS_KEY.to_string(),
            serde_json::Value::from(days),
        );
    })?;
    Ok(())
}

/// Remove the key so CC's 30-day default applies again.
///
/// This is also the repair for [`RetentionMode::LegacyZero`] — see
/// [`set_retention_days`] for why there is no longer a "disable
/// persistence" sibling to this function.
pub fn clear_retention() -> Result<(), RetentionError> {
    clear_bool_setting(SettingsLayer::User, Path::new(""), CLEANUP_PERIOD_DAYS_KEY)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value as JsonValue;
    use std::fs;
    use tempfile::TempDir;

    fn isolated() -> (TempDir, std::sync::MutexGuard<'static, ()>) {
        let lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("config-dir");
        std::env::set_var("CLAUDE_CONFIG_DIR", &cfg);
        fs::create_dir_all(&cfg).unwrap();
        (tmp, lock)
    }

    fn write_user_settings(body: &str) {
        let p = user_settings_path();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, body).unwrap();
    }

    // ─── resolution table ───────────────────────────────────────────

    #[test]
    fn absent_key_is_cc_default_thirty() {
        let s = state_from_configured(SettingValue::Absent);
        assert_eq!(s.mode, RetentionMode::CcDefault);
        assert_eq!(s.effective_days, CC_DEFAULT_CLEANUP_PERIOD_DAYS);
        assert!(s.is_cc_default);
    }

    /// `0` was a legal, maximally-destructive value through CC 2.1.88.
    /// CC 2.1.233 rejects it (floor of 1), which inverts its effect: the
    /// key is present and fails validation, so cleanup is suppressed and
    /// transcripts accumulate instead of being destroyed. A `0` left on
    /// disk by an older Claudepot is in exactly this state.
    #[test]
    fn zero_is_rejected_by_cc_and_suppresses_cleanup() {
        let s = state_from_configured(SettingValue::Present(0));
        assert_eq!(s.mode, RetentionMode::LegacyZero);
        assert!(
            s.cleanup_suppressed,
            "0 fails CC's minimum-1 schema with the key present, which is \
             precisely CC's documented skip condition"
        );
        assert_eq!(
            s.effective_days, CC_DEFAULT_CLEANUP_PERIOD_DAYS,
            "must not be 0 — cleanup is suppressed, so a 0 here would \
             invite a caller to render 'deletes after 0 days'"
        );
        assert!(!s.is_cc_default);
    }

    /// A value below CC's floor of 1 fails its schema.
    /// Cleanup then bails before deleting anything, because settings
    /// have errors AND the raw key is present — so an invalid value
    /// accidentally *protects* transcripts.
    #[test]
    fn negative_suppresses_cleanup_entirely() {
        let s = state_from_configured(SettingValue::Present(-1));
        assert_eq!(s.mode, RetentionMode::Invalid);
        assert!(s.cleanup_suppressed);
        assert!(!s.is_cc_default);
    }

    #[test]
    fn valid_modes_do_not_suppress_cleanup() {
        for cfg in [
            SettingValue::Absent,
            // `Present(0)` deliberately absent — no longer valid; see
            // `zero_is_rejected_by_cc_and_suppresses_cleanup`.
            SettingValue::Present(MIN_CLEANUP_PERIOD_DAYS),
            SettingValue::Present(30),
            SettingValue::Present(3650),
        ] {
            assert!(
                !state_from_configured(cfg).cleanup_suppressed,
                "cfg {cfg:?} must not read as cleanup-suppressed"
            );
        }
    }

    /// A value that is present but not an integer fails CC's schema exactly
    /// as a negative one does, so cleanup is suppressed — and this case used
    /// to read as `CcDefault`, telling the user a 30-day timer was running
    /// on history CC was in fact leaving alone.
    #[test]
    fn a_non_integer_value_suppresses_cleanup_and_is_not_the_cc_default() {
        let s = state_from_configured(SettingValue::Invalid);
        assert_eq!(s.mode, RetentionMode::Invalid);
        assert!(s.cleanup_suppressed);
        assert!(!s.is_cc_default, "must never advise restoring the default");
        assert_eq!(s.configured_days, None);
    }

    /// End to end through the reader, because the collapse that caused the
    /// bug lived there rather than in the resolution table.
    #[test]
    fn non_integer_values_on_disk_read_as_invalid_not_absent() {
        let (_t, _l) = isolated();
        for body in [
            r#"{"cleanupPeriodDays":"thirty"}"#,
            r#"{"cleanupPeriodDays":30.5}"#,
            r#"{"cleanupPeriodDays":true}"#,
            r#"{"cleanupPeriodDays":null}"#,
        ] {
            write_user_settings(body);
            let s = resolve_retention();
            assert_eq!(
                s.mode,
                RetentionMode::Invalid,
                "{body} should suppress cleanup"
            );
            assert!(s.cleanup_suppressed);
        }

        // …and the genuinely absent case still reads as the CC default.
        write_user_settings(r#"{"other":1}"#);
        assert_eq!(resolve_retention().mode, RetentionMode::CcDefault);
    }

    /// The dangerous consequence of the above: with cleanup suppressed,
    /// nothing is at risk however old it is. Reporting risk here would
    /// push the user toward "restore the default", which clears the
    /// error, re-arms cleanup, and deletes everything past 30 days.
    #[test]
    fn suppressed_cleanup_puts_nothing_at_risk() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        let now = 1_800_000_000_000i64;
        seed_projects(&root, &[400, 800], &[], now);
        let r = scan_transcript_risk_in(
            &root,
            &state_from_configured(SettingValue::Present(-1)),
            now,
            7,
        );
        assert_eq!(r.total_transcripts, 2);
        assert_eq!(r.already_deletable, 0);
        assert_eq!(r.at_risk_within_horizon, 0);
    }

    #[test]
    fn positive_is_explicit() {
        let s = state_from_configured(SettingValue::Present(3650));
        assert_eq!(s.mode, RetentionMode::Explicit);
        assert_eq!(s.effective_days, 3650);
        assert!(!s.is_cc_default);
    }

    // ─── write path ─────────────────────────────────────────────────

    #[test]
    fn set_and_clear_round_trip_preserving_other_keys() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"keep":1}"#);

        set_retention_days(3650).unwrap();
        assert_eq!(resolve_retention().configured_days, Some(3650));
        assert_eq!(resolve_retention().mode, RetentionMode::Explicit);

        clear_retention().unwrap();
        let v: JsonValue =
            serde_json::from_slice(&fs::read(user_settings_path()).unwrap()).unwrap();
        assert!(v.get(CLEANUP_PERIOD_DAYS_KEY).is_none());
        assert_eq!(v["keep"], JsonValue::from(1));
        assert_eq!(resolve_retention().mode, RetentionMode::CcDefault);
    }

    /// The whole point of the module: a slider cannot reach 0.
    #[test]
    fn set_retention_days_refuses_zero_and_negatives() {
        let (_t, _l) = isolated();
        assert!(matches!(
            set_retention_days(0),
            Err(RetentionError::NonPositive(0))
        ));
        assert!(matches!(
            set_retention_days(-5),
            Err(RetentionError::NonPositive(-5))
        ));
        // and nothing was written
        assert_eq!(resolve_retention().configured_days, None);
    }

    /// There is no route to `0` at all any more, from any public entry
    /// point in this module. `disable_persistence()` was deleted rather
    /// than repointed: current CC offers no settings-file way to disable
    /// transcript writes, so every possible implementation of that verb
    /// would write a value CC rejects and report success.
    ///
    /// This is the regression guard for re-adding one. If a future CC
    /// brings back a settings-level disable, this test should be
    /// replaced deliberately — not deleted because it started failing.
    #[test]
    fn no_public_writer_can_reach_zero() {
        let (_t, _l) = isolated();
        for d in [0, -1, i64::MIN] {
            assert!(set_retention_days(d).is_err(), "set_retention_days({d})");
        }
        assert_eq!(
            resolve_retention().configured_days,
            None,
            "no failed write may leave a value behind"
        );
        // The one remaining writer that touches the key at all removes
        // it, which is also the repair for a legacy zero.
        set_retention_days(90).unwrap();
        clear_retention().unwrap();
        assert_eq!(resolve_retention().mode, RetentionMode::CcDefault);
    }

    /// This test used to assert the opposite — that a non-integer value
    /// "reads as unset" — and in doing so locked in the bug. CC skips
    /// cleanup when its settings fail validation and this key is present
    /// (`utils/cleanup.ts:579`), and `z.number().int()` rejects `"thirty"`
    /// exactly as it rejects `-1`. Reporting `CcDefault` told the user a
    /// 30-day timer was running on history CC was leaving alone, and pointed
    /// them at "restore the default" — the one action that starts it.
    #[test]
    fn a_non_integer_value_suppresses_cleanup_rather_than_reading_as_unset() {
        let (_t, _l) = isolated();
        write_user_settings(r#"{"cleanupPeriodDays":"thirty"}"#);
        let s = resolve_retention();
        assert_eq!(s.mode, RetentionMode::Invalid);
        assert!(s.cleanup_suppressed);
        assert!(!s.is_cc_default);
    }

    // ─── risk scan ──────────────────────────────────────────────────

    /// Build `projects/<slug>/` with files at given ages in days.
    fn seed_projects(root: &Path, top_level_ages: &[i64], nested_ages: &[i64], now_ms: i64) {
        let proj = root.join("-Users-joker-demo");
        fs::create_dir_all(&proj).unwrap();
        for (i, age) in top_level_ages.iter().enumerate() {
            let p = proj.join(format!("session-{i}.jsonl"));
            fs::write(&p, b"{}\n").unwrap();
            set_mtime(&p, now_ms - age * MS_PER_DAY);
        }
        let sub = proj.join("0000-session").join("subagents");
        fs::create_dir_all(&sub).unwrap();
        for (i, age) in nested_ages.iter().enumerate() {
            let p = sub.join(format!("agent-{i}.jsonl"));
            fs::write(&p, b"{}\n").unwrap();
            set_mtime(&p, now_ms - age * MS_PER_DAY);
        }
    }

    fn set_mtime(path: &Path, ms: i64) {
        let t = filetime::FileTime::from_unix_time(ms / 1000, ((ms % 1000) * 1_000_000) as u32);
        filetime::set_file_mtime(path, t).unwrap();
    }

    #[test]
    fn counts_only_top_level_files_as_deletable() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        let now = 1_800_000_000_000i64;
        // three top-level: 60d (past cutoff), 27d (within 7d horizon), 1d (safe)
        // two nested at 400d — cleanup never walks these
        seed_projects(&root, &[60, 27, 1], &[400, 400], now);

        let state = state_from_configured(SettingValue::Absent); // 30-day default
        let r = scan_transcript_risk_in(&root, &state, now, 7);

        assert_eq!(
            r.total_transcripts, 3,
            "nested files are not transcripts CC deletes"
        );
        assert_eq!(r.already_deletable, 1, "the 60-day file is past the cutoff");
        assert_eq!(
            r.at_risk_within_horizon, 1,
            "the 27-day file crosses within 7 days"
        );
        assert_eq!(r.nested_immortal, 2, "subagent files survive forever");
    }

    /// The reason the loss is invisible: the immortal nested files are
    /// the bulk, so directory size rises while transcripts are deleted.
    #[test]
    fn nested_immortal_is_reported_separately_from_deletable() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        let now = 1_800_000_000_000i64;
        seed_projects(&root, &[400], &[400; 20], now);
        let r =
            scan_transcript_risk_in(&root, &state_from_configured(SettingValue::Absent), now, 7);
        assert_eq!(r.already_deletable, 1);
        assert_eq!(r.nested_immortal, 20);
    }

    #[test]
    fn long_retention_puts_nothing_at_risk() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        let now = 1_800_000_000_000i64;
        seed_projects(&root, &[60, 27, 1], &[], now);
        let r = scan_transcript_risk_in(
            &root,
            &state_from_configured(SettingValue::Present(3650)),
            now,
            7,
        );
        assert_eq!(r.already_deletable, 0);
        assert_eq!(r.at_risk_within_horizon, 0);
        assert_eq!(r.total_transcripts, 3);
    }

    /// Previously `persistence_disabled_puts_everything_at_risk`, which
    /// asserted all three files were already deletable because a `0`
    /// window put the cutoff at `now`. Under current CC the assertion
    /// inverts: `0` is rejected, so cleanup is suppressed and *nothing*
    /// is at risk however old it is — the scan pushes the cutoff to the
    /// floor for exactly this case, same as [`RetentionMode::Invalid`].
    ///
    /// The transcripts are still counted, because the pane must be able
    /// to say how much history is accumulating while the user believes
    /// persistence is off.
    #[test]
    fn legacy_zero_scans_like_invalid_and_deletes_nothing() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        let now = 1_800_000_000_000i64;
        seed_projects(&root, &[60, 27, 1], &[], now);
        let state = state_from_configured(SettingValue::Present(0));
        let r = scan_transcript_risk_in(&root, &state, now, 7);

        assert!(state.cleanup_suppressed);
        assert_eq!(
            r.already_deletable, 0,
            "cleanup is suppressed, so the 60-day file is not on any timer"
        );
        assert_eq!(r.at_risk_within_horizon, 0);
        assert_eq!(r.total_transcripts, 3, "but they are still counted");

        // And the expiring boot check must stay silent: warning about
        // deletion that is not happening is the failure this guards.
        let report = RetentionReport {
            state,
            risk: r,
            is_durable_archive: false,
        };
        assert!(warning(&report).is_none());
        // ...but the *suppressed* check must fire, or the state is
        // invisible outside the pane.
        assert!(cleanup_suppressed_warning(&report).is_some());
    }

    fn report_with(
        mode_source: SettingValue<i64>,
        total: u64,
        incomplete: bool,
    ) -> RetentionReport {
        RetentionReport {
            state: state_from_configured(mode_source),
            risk: TranscriptRisk {
                total_transcripts: total,
                scan_incomplete: incomplete,
                ..Default::default()
            },
            is_durable_archive: false,
        }
    }

    /// The two warnings partition the space: exactly one of them can
    /// speak for any given state. If both could fire, the pane's user
    /// would get "conversations are expiring" and "cleanup is switched
    /// off" in the same bell, which is incoherent.
    #[test]
    fn the_two_boot_warnings_are_mutually_exclusive() {
        for cfg in [
            SettingValue::Absent,
            SettingValue::Present(0),
            SettingValue::Present(-1),
            SettingValue::Present(30),
            SettingValue::Invalid,
        ] {
            let rep = report_with(cfg, 12, false);
            let expiring = warning(&rep).is_some();
            let suppressed = cleanup_suppressed_warning(&rep).is_some();
            assert!(
                !(expiring && suppressed),
                "cfg {cfg:?} produced both warnings"
            );
            assert_eq!(
                suppressed, rep.state.cleanup_suppressed,
                "cfg {cfg:?}: the suppressed warning must track the flag exactly"
            );
        }
    }

    #[test]
    fn a_legacy_zero_says_transcripts_are_being_kept_not_deleted() {
        let w = cleanup_suppressed_warning(&report_with(SettingValue::Present(0), 460, false))
            .expect("legacy zero must warn");
        assert_eq!(w.mode, RetentionMode::LegacyZero);
        assert_eq!(w.configured_days, Some(0));
        let m = w.message();
        assert!(m.contains("460"), "{m}");
        assert!(m.contains("keeping every saved conversation"), "{m}");
        // The inversion is the whole point: never tell this user their
        // transcripts are being deleted.
        assert!(!m.contains("will delete"), "{m}");
    }

    #[test]
    fn an_invalid_value_says_the_protection_is_accidental() {
        let w = cleanup_suppressed_warning(&report_with(SettingValue::Invalid, 7, false))
            .expect("invalid must warn");
        assert_eq!(w.mode, RetentionMode::Invalid);
        assert_eq!(w.configured_days, None);
        let m = w.message();
        assert!(m.contains("accidental"), "{m}");
        // Must never suggest restoring the default — that re-arms
        // deletion, which is the trap this whole module exists for.
        assert!(!m.to_lowercase().contains("restore"), "{m}");
    }

    /// An unread tree makes the count a floor. Reporting it as a total
    /// is the same class of error as reporting a failed scan as "nothing
    /// scheduled for deletion".
    #[test]
    fn an_incomplete_scan_reports_the_count_as_a_floor() {
        let w = cleanup_suppressed_warning(&report_with(SettingValue::Present(0), 3, true))
            .expect("must warn");
        assert!(w.message().contains("at least 3"), "{}", w.message());
    }

    #[test]
    fn oldest_ms_reports_the_earliest_surviving_transcript() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        let now = 1_800_000_000_000i64;
        seed_projects(&root, &[60, 27, 1], &[], now);
        let r =
            scan_transcript_risk_in(&root, &state_from_configured(SettingValue::Absent), now, 7);
        assert_eq!(r.oldest_ms, Some(now - 60 * MS_PER_DAY));
    }

    // ─── scan completeness ──────────────────────────────────────────

    /// The worst failure this feature could have: an unreadable tree
    /// rendering as "nothing is scheduled for deletion".
    #[cfg(unix)]
    #[test]
    fn unreadable_projects_dir_is_incomplete_not_safe() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o000)).unwrap();

        let r = scan_transcript_risk_in(
            &root,
            &state_from_configured(SettingValue::Absent),
            1_800_000_000_000,
            7,
        );

        // Restore before assertions so a failure doesn't leave an
        // unremovable temp dir behind.
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            r.scan_incomplete,
            "an unreadable tree must not read as safe"
        );
        assert_eq!(r.already_deletable, 0);
    }

    /// ...and the boot check must speak up about it rather than
    /// staying silent on zero counts.
    #[test]
    fn incomplete_scan_still_warns() {
        let r = RetentionReport {
            state: state_from_configured(SettingValue::Absent),
            risk: TranscriptRisk {
                scan_incomplete: true,
                horizon_days: 7,
                ..Default::default()
            },
            is_durable_archive: false,
        };
        let w = warning(&r).expect("incomplete scan must warn");
        assert!(w.scan_incomplete);
        assert!(w.message().contains("could not read"));
    }

    /// Regression: an incomplete scan must not produce a "conversations
    /// are expiring" warning on a machine where CC's cleanup is
    /// switched off entirely — that alarms the user about the one thing
    /// that cannot be happening.
    #[test]
    fn suppressed_cleanup_never_warns_even_on_an_incomplete_scan() {
        let r = RetentionReport {
            state: state_from_configured(SettingValue::Present(-1)),
            risk: TranscriptRisk {
                scan_incomplete: true,
                horizon_days: 7,
                ..Default::default()
            },
            is_durable_archive: false,
        };
        assert!(r.state.cleanup_suppressed);
        assert!(warning(&r).is_none());
    }

    #[test]
    fn a_complete_empty_scan_is_not_incomplete() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        fs::create_dir_all(&root).unwrap();
        let r = scan_transcript_risk_in(
            &root,
            &state_from_configured(SettingValue::Absent),
            1_800_000_000_000,
            7,
        );
        assert!(!r.scan_incomplete);
    }

    // ─── arithmetic safety ──────────────────────────────────────────

    /// `now_ms` and `effective_days` both arrive from outside (a clock
    /// and a user-editable file). Neither may panic the scan.
    #[test]
    fn extreme_values_saturate_instead_of_overflowing() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        let now = 1_800_000_000_000i64;
        seed_projects(&root, &[1], &[], now);

        for (days, horizon, now_ms) in [
            (i64::MAX, 7, now),
            (i64::MAX, i64::MAX, now),
            (1, 7, i64::MAX),
            (1, 7, i64::MIN),
            (i64::MAX, i64::MAX, i64::MAX),
        ] {
            let st = state_from_configured(SettingValue::Present(days));
            // Must not panic in debug (where overflow aborts).
            let _ = scan_transcript_risk_in(&root, &st, now_ms, horizon);
        }
    }

    #[test]
    fn missing_projects_dir_is_zero_not_an_error() {
        let tmp = TempDir::new().unwrap();
        let r = scan_transcript_risk_in(
            &tmp.path().join("nope"),
            &state_from_configured(SettingValue::Absent),
            1_800_000_000_000,
            7,
        );
        assert_eq!(
            r,
            TranscriptRisk {
                horizon_days: 7,
                ..Default::default()
            }
        );
    }

    // ─── boot warning ───────────────────────────────────────────────

    fn rep(state: RetentionState, risk: TranscriptRisk) -> RetentionReport {
        RetentionReport {
            state,
            risk,
            is_durable_archive: false,
        }
    }

    #[test]
    fn no_warning_when_nothing_is_at_risk() {
        let r = rep(
            state_from_configured(SettingValue::Absent),
            TranscriptRisk {
                total_transcripts: 460,
                horizon_days: 7,
                ..Default::default()
            },
        );
        assert!(warning(&r).is_none());
    }

    /// An unset setting is not by itself a reason to nag — only actual
    /// scheduled deletion is.
    #[test]
    fn cc_default_alone_does_not_warn() {
        let r = rep(
            state_from_configured(SettingValue::Absent),
            TranscriptRisk::default(),
        );
        assert!(warning(&r).is_none());
    }

    #[test]
    fn warns_when_transcripts_are_already_past_the_cutoff() {
        let r = rep(
            state_from_configured(SettingValue::Absent),
            TranscriptRisk {
                already_deletable: 1247,
                horizon_days: 7,
                ..Default::default()
            },
        );
        let w = warning(&r).expect("should warn");
        assert_eq!(w.already_deletable, 1247);
        assert!(w.message().contains("1247"));
        assert!(w.message().contains("nobody chose"));
    }

    #[test]
    fn warns_on_upcoming_risk_even_when_none_are_past_yet() {
        let r = rep(
            state_from_configured(SettingValue::Present(30)),
            TranscriptRisk {
                at_risk_within_horizon: 3,
                horizon_days: 7,
                ..Default::default()
            },
        );
        let w = warning(&r).expect("should warn");
        assert_eq!(w.already_deletable, 0);
        assert!(w.message().contains("within 7 days"));
        // Explicitly chosen, so we don't editorialise about defaults.
        assert!(!w.message().contains("nobody chose"));
    }

    /// The dismissal requirement, satisfied by gating on the condition
    /// rather than persisting a flag: raise retention, and the warning
    /// stops on its own.
    #[test]
    fn raising_retention_silences_the_warning_without_a_dismiss_flag() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        let now = 1_800_000_000_000i64;
        seed_projects(&root, &[60, 27, 1], &[], now);

        let before = state_from_configured(SettingValue::Absent);
        let r_before = rep(
            before.clone(),
            scan_transcript_risk_in(&root, &before, now, 7),
        );
        assert!(warning(&r_before).is_some());

        let after = state_from_configured(SettingValue::Present(3650));
        let r_after = rep(
            after.clone(),
            scan_transcript_risk_in(&root, &after, now, 7),
        );
        assert!(warning(&r_after).is_none());
    }

    #[test]
    fn singular_plural_reads_correctly() {
        let one = rep(
            state_from_configured(SettingValue::Present(30)),
            TranscriptRisk {
                already_deletable: 1,
                horizon_days: 7,
                ..Default::default()
            },
        );
        assert!(warning(&one)
            .unwrap()
            .message()
            .contains("1 saved conversation on"));
    }

    /// `.cast` recordings are unlinked by the same arm of CC's cleanup.
    #[test]
    fn cast_files_count_as_deletable_transcripts() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("projects");
        let now = 1_800_000_000_000i64;
        let proj = root.join("-Users-joker-demo");
        fs::create_dir_all(&proj).unwrap();
        let p = proj.join("rec.cast");
        fs::write(&p, b"x").unwrap();
        set_mtime(&p, now - 60 * MS_PER_DAY);
        let other = proj.join("notes.md");
        fs::write(&other, b"x").unwrap();
        set_mtime(&other, now - 60 * MS_PER_DAY);

        let r =
            scan_transcript_risk_in(&root, &state_from_configured(SettingValue::Absent), now, 7);
        assert_eq!(r.total_transcripts, 1, "only .jsonl/.cast are cleaned");
        assert_eq!(r.already_deletable, 1);
    }
}
