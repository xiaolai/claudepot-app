//! Usage-threshold crossing detector + persistent alert state.
//!
//! Goal: fire one OS notification per (account × window × threshold)
//! per reset cycle. The state file remembers which thresholds were
//! already fired this cycle; when the window resets (different
//! `resets_at` timestamp), the fired set clears and the next
//! crossing fires again.
//!
//! Why a fired-set instead of a prev/now comparison: it directly
//! answers "did we fire this already for this cycle?" without
//! relying on consecutive-poll prev values. If a poll is skipped
//! and utilization jumps 50% → 95%, the 80% threshold should
//! still fire — the user didn't get the early warning yet.

use crate::oauth::usage::{UsageResponse, UsageWindow};
use crate::paths;
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

/// Stable identifier for the four usage windows we surface. Maps to
/// the corresponding `Option<UsageWindow>` field on `UsageResponse`.
/// `extra_usage` is its own quota model (monthly $ cap) and lives on a
/// separate variant carrying the cap rather than a utilization %.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageWindowKind {
    FiveHour,
    SevenDay,
    SevenDayOpus,
    SevenDaySonnet,
}

impl UsageWindowKind {
    /// Short label for the notification copy. Kept terse — the
    /// account email and percentage carry the rest of the meaning.
    pub fn label(self) -> &'static str {
        match self {
            Self::FiveHour => "5-hour window",
            Self::SevenDay => "7-day window",
            Self::SevenDayOpus => "7-day Opus window",
            Self::SevenDaySonnet => "7-day Sonnet window",
        }
    }

    fn pick<'a>(self, resp: &'a UsageResponse) -> Option<&'a UsageWindow> {
        match self {
            Self::FiveHour => resp.five_hour.as_ref(),
            Self::SevenDay => resp.seven_day.as_ref(),
            Self::SevenDayOpus => resp.seven_day_opus.as_ref(),
            Self::SevenDaySonnet => resp.seven_day_sonnet.as_ref(),
        }
    }

    /// All windows the watcher checks. Order matters only insofar as
    /// the resulting Vec<Crossing> is iterated to fire notifications.
    pub fn all() -> [Self; 4] {
        [
            Self::FiveHour,
            Self::SevenDay,
            Self::SevenDayOpus,
            Self::SevenDaySonnet,
        ]
    }
}

/// One newly-crossed threshold. The watcher emits one of these per
/// detected crossing in a poll cycle; the Tauri layer translates each
/// into an OS notification + frontend event.
#[derive(Debug, Clone, PartialEq)]
pub struct Crossing {
    pub account_uuid: Uuid,
    pub window: UsageWindowKind,
    pub threshold_pct: u32,
    pub utilization_pct: f64,
    pub resets_at: Option<DateTime<FixedOffset>>,
}

/// Per-window persisted state — which `resets_at` cycle we last saw
/// and which thresholds we've already fired for that cycle.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct WindowState {
    /// The `resets_at` timestamp of the cycle whose `fired` we are
    /// tracking. `None` covers two cases: never observed, OR the
    /// server returned a window with no reset (utilization=0,
    /// resets_at=null on accounts that haven't activated the window
    /// yet). When the freshly-observed `resets_at` differs from this
    /// value, we treat it as a new cycle and clear `fired`.
    last_resets_at: Option<DateTime<FixedOffset>>,
    /// Thresholds (as integer %) already fired during the current
    /// cycle. Cleared when `last_resets_at` advances. Stored sorted
    /// + deduped for stable serialization, but membership semantics
    /// is what the algorithm actually relies on.
    fired: Vec<u32>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct AccountState {
    /// Per-window state. Missing entry == never observed; equivalent
    /// to a default-constructed `WindowState`.
    #[serde(default)]
    windows: HashMap<UsageWindowKind, WindowState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AlertStateFile {
    /// Bumped on schema changes; `load` accepts only the current
    /// version and starts fresh on mismatch.
    version: u32,
    #[serde(default)]
    accounts: HashMap<Uuid, AccountState>,
}

impl Default for AlertStateFile {
    fn default() -> Self {
        Self {
            version: STATE_FILE_VERSION,
            accounts: HashMap::new(),
        }
    }
}

const STATE_FILE_VERSION: u32 = 1;

/// In-memory mirror of the on-disk state. Mutate via `apply_crossings`
/// then call `save` to persist. Reads/writes are blocking; the
/// caller decides whether to wrap in `spawn_blocking`.
///
/// Cloneable so callers (e.g. the Tauri usage_watcher task) can take
/// an owned snapshot before handing it to `spawn_blocking` for the
/// disk write — the file is tiny (one entry per account × window),
/// so the per-save clone is cheap.
#[derive(Debug, Default, Clone)]
pub struct UsageAlertState {
    file: AlertStateFile,
}

impl UsageAlertState {
    pub fn new() -> Self {
        Self::default()
    }

    fn path() -> PathBuf {
        paths::claudepot_data_dir().join("usage_alert_state.json")
    }

    /// Load from disk. Missing file or mismatched schema version →
    /// fresh empty state; never errors. The intent is "one notification
    /// per (window × threshold) per cycle most of the time" —
    /// occasionally re-firing after a corrupt file is far less bad
    /// than silently losing all alerts because of a parse error.
    pub fn load() -> Self {
        let p = Self::path();
        let raw = match std::fs::read_to_string(&p) {
            Ok(s) => s,
            Err(_) => return Self::default(),
        };
        let parsed: AlertStateFile = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(path = %p.display(), error = %e, "usage_alert_state: parse failed, starting fresh");
                return Self::default();
            }
        };
        if parsed.version != STATE_FILE_VERSION {
            tracing::info!(
                got = parsed.version,
                want = STATE_FILE_VERSION,
                "usage_alert_state: schema version mismatch, starting fresh"
            );
            return Self::default();
        }
        Self { file: parsed }
    }

    /// Persist to disk. The state file is small (one entry per
    /// account × window) and rewritten in full each time. We do NOT
    /// fsync — losing the most recent write means the next cycle
    /// might re-fire a threshold, which is benign.
    pub fn save(&self) -> std::io::Result<()> {
        let p = Self::path();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let s = serde_json::to_string_pretty(&self.file).map_err(std::io::Error::other)?;
        std::fs::write(&p, s)
    }

    /// Compute and record crossings for one account's response. Any
    /// crossings detected are returned AND folded into the in-memory
    /// state; the caller is responsible for calling `save` afterwards.
    /// `thresholds` should be sorted ascending and deduped; the
    /// algorithm itself doesn't rely on order, but the consumer's
    /// notification copy reads cleaner that way.
    pub fn apply_crossings(
        &mut self,
        account_uuid: Uuid,
        resp: &UsageResponse,
        thresholds: &[u32],
    ) -> Vec<Crossing> {
        let acct = self.file.accounts.entry(account_uuid).or_default();
        let mut out = Vec::new();
        for kind in UsageWindowKind::all() {
            let window = match kind.pick(resp) {
                Some(w) => w,
                None => continue,
            };
            // No reset timestamp means the window isn't active yet
            // (Anthropic returns `resets_at: null` on cold accounts).
            // Skip — it can't have crossed anything meaningful.
            let resets = match window.resets_at {
                Some(r) => r,
                None => continue,
            };
            let entry = acct.windows.entry(kind).or_default();
            // Cycle boundary: clear the fired set when the reset
            // timestamp advances (or first observation flips it from
            // None → Some).
            if entry.last_resets_at != Some(resets) {
                entry.last_resets_at = Some(resets);
                entry.fired.clear();
            }
            // Server returns utilization as a percent in 0..=100 with
            // floating-point precision. Round half-down for the
            // comparison so 79.999 doesn't trip 80; the user's mental
            // model is "the percent shown in the UI."
            let pct = window.utilization;
            for &t in thresholds {
                if pct >= t as f64 && !entry.fired.contains(&t) {
                    entry.fired.push(t);
                    out.push(Crossing {
                        account_uuid,
                        window: kind,
                        threshold_pct: t,
                        utilization_pct: pct,
                        resets_at: Some(resets),
                    });
                }
            }
            // Keep `fired` sorted+deduped for stable serialization
            // and so the file is readable at a glance.
            entry.fired.sort_unstable();
            entry.fired.dedup();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_window(util: f64, resets: Option<&str>) -> UsageWindow {
        UsageWindow {
            utilization: util,
            resets_at: resets.map(|s| DateTime::parse_from_rfc3339(s).unwrap()),
        }
    }

    fn response_with_five_hour(util: f64, resets: Option<&str>) -> UsageResponse {
        UsageResponse {
            five_hour: Some(make_window(util, resets)),
            seven_day: None,
            seven_day_oauth_apps: None,
            seven_day_opus: None,
            seven_day_sonnet: None,
            seven_day_cowork: None,
            iguana_necktie: None,
            extra_usage: None,
            unknown: HashMap::new(),
        }
    }

    #[test]
    fn first_observation_below_thresholds_fires_nothing() {
        let mut st = UsageAlertState::new();
        let uuid = Uuid::new_v4();
        let r = response_with_five_hour(42.0, Some("2026-04-30T10:00:00+00:00"));
        let crossings = st.apply_crossings(uuid, &r, &[80, 90]);
        assert!(crossings.is_empty());
    }

    #[test]
    fn crossing_80_fires_once_per_cycle() {
        let mut st = UsageAlertState::new();
        let uuid = Uuid::new_v4();
        let resets = "2026-04-30T10:00:00+00:00";
        // First poll under threshold.
        let _ = st.apply_crossings(uuid, &response_with_five_hour(70.0, Some(resets)), &[80, 90]);
        // Second poll crosses 80.
        let crossings = st.apply_crossings(
            uuid,
            &response_with_five_hour(82.5, Some(resets)),
            &[80, 90],
        );
        assert_eq!(crossings.len(), 1);
        assert_eq!(crossings[0].threshold_pct, 80);
        assert_eq!(crossings[0].window, UsageWindowKind::FiveHour);
        // Third poll still ≥80 but same cycle — must not refire.
        let crossings = st.apply_crossings(
            uuid,
            &response_with_five_hour(85.0, Some(resets)),
            &[80, 90],
        );
        assert!(crossings.is_empty());
    }

    #[test]
    fn jump_past_two_thresholds_fires_both() {
        let mut st = UsageAlertState::new();
        let uuid = Uuid::new_v4();
        let resets = "2026-04-30T10:00:00+00:00";
        // Single poll, utilization shoots from 0 to 95 (e.g. cold start
        // followed by the user using the API hard).
        let crossings = st.apply_crossings(
            uuid,
            &response_with_five_hour(95.0, Some(resets)),
            &[80, 90],
        );
        let firing: Vec<u32> = crossings.iter().map(|c| c.threshold_pct).collect();
        assert_eq!(firing, vec![80, 90]);
    }

    #[test]
    fn cycle_reset_rearms_fired_thresholds() {
        let mut st = UsageAlertState::new();
        let uuid = Uuid::new_v4();
        // Cycle 1: cross 80.
        let _ = st.apply_crossings(
            uuid,
            &response_with_five_hour(85.0, Some("2026-04-30T10:00:00+00:00")),
            &[80, 90],
        );
        // Cycle 2: same utilization, new reset timestamp → re-fires.
        let crossings = st.apply_crossings(
            uuid,
            &response_with_five_hour(85.0, Some("2026-04-30T15:00:00+00:00")),
            &[80, 90],
        );
        assert_eq!(crossings.len(), 1);
        assert_eq!(crossings[0].threshold_pct, 80);
    }

    #[test]
    fn null_resets_at_skips_window() {
        let mut st = UsageAlertState::new();
        let uuid = Uuid::new_v4();
        // Window present but inactive (resets_at None).
        let crossings = st.apply_crossings(uuid, &response_with_five_hour(99.0, None), &[80, 90]);
        assert!(crossings.is_empty(), "inactive windows must not fire");
    }

    #[test]
    fn empty_thresholds_fires_nothing() {
        let mut st = UsageAlertState::new();
        let uuid = Uuid::new_v4();
        let crossings = st.apply_crossings(
            uuid,
            &response_with_five_hour(99.0, Some("2026-04-30T10:00:00+00:00")),
            &[],
        );
        assert!(crossings.is_empty());
    }

    #[test]
    fn distinct_accounts_have_isolated_state() {
        let mut st = UsageAlertState::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let resets = "2026-04-30T10:00:00+00:00";
        // Account A crosses 80.
        let _ = st.apply_crossings(a, &response_with_five_hour(85.0, Some(resets)), &[80]);
        // Account B at the same utilization must still fire (different
        // account, different state).
        let crossings = st.apply_crossings(b, &response_with_five_hour(85.0, Some(resets)), &[80]);
        assert_eq!(crossings.len(), 1);
        assert_eq!(crossings[0].account_uuid, b);
    }

    #[test]
    fn multi_window_response_fires_per_window() {
        let mut st = UsageAlertState::new();
        let uuid = Uuid::new_v4();
        let resets = "2026-04-30T10:00:00+00:00";
        let r = UsageResponse {
            five_hour: Some(make_window(85.0, Some(resets))),
            seven_day: Some(make_window(82.0, Some(resets))),
            seven_day_oauth_apps: None,
            seven_day_opus: None,
            seven_day_sonnet: None,
            seven_day_cowork: None,
            iguana_necktie: None,
            extra_usage: None,
            unknown: HashMap::new(),
        };
        let crossings = st.apply_crossings(uuid, &r, &[80]);
        // Both windows over 80 → two crossings.
        assert_eq!(crossings.len(), 2);
        let kinds: Vec<UsageWindowKind> = crossings.iter().map(|c| c.window).collect();
        assert!(kinds.contains(&UsageWindowKind::FiveHour));
        assert!(kinds.contains(&UsageWindowKind::SevenDay));
    }

    #[test]
    fn fired_list_is_sorted_and_deduped() {
        let mut st = UsageAlertState::new();
        let uuid = Uuid::new_v4();
        let resets = "2026-04-30T10:00:00+00:00";
        // Threshold list passed out-of-order; algorithm must still
        // produce a sorted+deduped fired list afterwards.
        let _ = st.apply_crossings(uuid, &response_with_five_hour(95.0, Some(resets)), &[90, 80]);
        let acct = st.file.accounts.get(&uuid).unwrap();
        let fired = &acct
            .windows
            .get(&UsageWindowKind::FiveHour)
            .unwrap()
            .fired;
        assert_eq!(fired, &vec![80, 90]);
    }

    #[test]
    fn label_strings_match_the_window_set() {
        // Lock the labels — they appear in user-visible toast copy
        // and changing them silently is a regression risk.
        assert_eq!(UsageWindowKind::FiveHour.label(), "5-hour window");
        assert_eq!(UsageWindowKind::SevenDay.label(), "7-day window");
        assert_eq!(UsageWindowKind::SevenDayOpus.label(), "7-day Opus window");
        assert_eq!(
            UsageWindowKind::SevenDaySonnet.label(),
            "7-day Sonnet window"
        );
    }
}
