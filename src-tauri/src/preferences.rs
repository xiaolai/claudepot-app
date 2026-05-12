//! Persisted UI preferences — a tiny JSON file in the Claudepot data dir.
//!
//! Read synchronously from `setup()` before the webview window is shown
//! so `hide_dock_icon` can flip the activation policy early enough to
//! avoid a visible dock-icon flash on cold launch. Any preference that
//! the CLI doesn't care about belongs here, not in `claudepot-core`.

use claudepot_core::config_view::model::EditorDefaults;
use claudepot_core::notifications::Category;
use claudepot_core::paths;
use claudepot_core::pricing::PriceTier;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// Per-category notification preference. Lives here (src-tauri),
/// NOT in claudepot-core — per `.claude/rules/architecture.md`,
/// GUI preferences stay GUI-side. The `Category` enum is in core
/// so the rules engine can reference categories abstractly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CategoryPrefs {
    /// Master toggle. `false` ⇒ emit() yields `surfaces_requested =
    /// []`; the event still logs (the routing's `log` field is
    /// independent of `enabled`) so the bell records a forensic
    /// trail of suppressed notifications.
    pub enabled: bool,
    /// Override the OS-banner surface specifically. `None` ⇒ follow
    /// category priority default. `Some(true)` ⇒ force OS on even
    /// for P2/P3. `Some(false)` ⇒ force OS off even for P0/P1.
    /// Independent of `enabled`.
    #[serde(default)]
    pub os_override: Option<bool>,
}

impl Default for CategoryPrefs {
    /// Generic fallback — kept for serde compatibility with hand-
    /// edited preferences.json entries that lack the `enabled`
    /// field. The runtime `category_pref()` getter uses the
    /// category-aware [`default_prefs_for`] instead so each
    /// category honors its `display_meta().default_enabled` policy
    /// (e.g. `SessionStuck`, `OpDoneUnfocused`, and
    /// `SessionErrorBurst` default OFF per the audit-validated
    /// activity-feature contract).
    fn default() -> Self {
        Self {
            enabled: true,
            os_override: None,
        }
    }
}

/// Build the canonical default `CategoryPrefs` for `category`.
/// Reads `Category::display_meta().default_enabled` so the runtime
/// fallback honors each category's documented default policy.
pub fn default_prefs_for(category: Category) -> CategoryPrefs {
    CategoryPrefs {
        enabled: category.display_meta().default_enabled,
        os_override: None,
    }
}

/// Effective surface request set for `category` under the current
/// user preferences. Audit-fix High #6: Rust-originated watchers
/// (usage_watcher, service_status_watcher) used to construct
/// `surfaces_requested` from a sub-set of legacy scalars, bypassing
/// `CategoryPrefs.enabled` and the `os_override` toggle that the
/// renderer-side emit() honors.
///
/// This helper centralizes the policy so both paths agree:
///
///   * `category_prefs(c).enabled == false` ⇒ `[]` (log-only).
///   * `enabled == true` and `wants_os` is true OR
///     `category_prefs(c).os_override == Some(true)` ⇒ `[OsBanner]`.
///   * `enabled == true` and OS is off ⇒ `[]`.
///
/// `wants_os` reflects the priority default the watcher would
/// otherwise apply (e.g. P1 categories want OS by default; P3
/// categories don't). The user's `os_override` flips it.
pub fn effective_os_surface(
    prefs: &Preferences,
    category: Category,
    wants_os: bool,
) -> Vec<claudepot_core::notifications::Surface> {
    let cp = prefs.category_pref(category);
    if !cp.enabled {
        return Vec::new();
    }
    let os = match cp.os_override {
        Some(v) => v,
        None => wants_os,
    };
    if os {
        vec![claudepot_core::notifications::Surface::OsBanner]
    } else {
        Vec::new()
    }
}

/// Current schema version for `Preferences`. Phase 1.5 of the
/// notification refactor bumps from 0 → 1, migrating the seven
/// scalar `notify_on_*` fields into `category_prefs`. The old
/// scalars are NOT removed in this version — dual-write keeps a
/// downgrade path open for one minor release. A follow-up release
/// bumps to 2 and drops them.
pub const PREFS_SCHEMA_VERSION_CURRENT: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// macOS-only. When true, the app runs as an accessory: no dock
    /// icon, no Cmd+Tab entry, no application menu bar. Tray-only.
    pub hide_dock_icon: bool,

    /// When false, the main window starts hidden on app launch. The
    /// user re-opens it via the tray icon. Pairs with `Launch at
    /// login` for a quiet tray-only background. Defaults to true so
    /// existing users keep seeing the window appear at start-up.
    #[serde(default = "default_true")]
    pub show_window_on_startup: bool,

    /// Whether the user has enabled the live Activity feature. Gate
    /// for starting the `LiveRuntime`: false until the consent modal
    /// is accepted. Defaults to false — no PID files or transcripts
    /// are read until the user opts in.
    pub activity_enabled: bool,

    /// Whether the user has seen (and dismissed) the first-run
    /// consent modal. Separate from `activity_enabled` so a user who
    /// declined once doesn't get re-prompted; they can opt in later
    /// from Settings. Defaults to false — modal shows on first run.
    pub activity_consent_seen: bool,

    /// When true, thinking blocks render as "▸ redacted · N chars"
    /// until the user explicitly clicks to reveal. Defaults to true
    /// — privacy-forward per the plan.
    #[serde(default = "default_true")]
    pub activity_hide_thinking: bool,

    /// Project paths the user has asked the live runtime to ignore.
    /// Compared as path-prefix matches against `PidRecord.cwd`.
    pub activity_excluded_paths: Vec<String>,

    /// Per-trigger notification toggles. Most default off; the one
    /// exception is `notify_on_waiting` (see below). M5 wires these
    /// to the tauri-plugin-notification backend.
    pub notify_on_error: bool,
    pub notify_on_idle_done: bool,
    /// None = feature off; Some(N) = fire after N minutes stuck.
    pub notify_on_stuck_minutes: Option<u32>,
    /// Fires an OS notification when a long-running op (verify_all,
    /// project rename, session prune/slim/share, account login/register,
    /// clean projects) terminates while the main window is unfocused.
    /// Default off — opt-in. The window-focus gate lives in the
    /// frontend dispatcher (`src/lib/notify.ts`).
    pub notify_on_op_done: bool,
    /// Fires when a session transitions into `Waiting` — CC has paused
    /// pending a permission, plan-mode approval, or clarifying answer.
    /// Defaults to **true** because this is the highest-leverage alert
    /// (a CLI the user can't see has stalled waiting on them) and the
    /// activity feature itself is already opt-in (`activity_enabled`),
    /// so a fresh-install user only sees these toasts after consenting
    /// to live tracking.
    #[serde(default = "default_true")]
    pub notify_on_waiting: bool,

    /// Anthropic usage-window utilization thresholds (integer percent
    /// values) that fire OS notifications when the CLI-active account
    /// crosses them. Empty vec = feature off; the default `[90]`
    /// gives one near-cap nudge per (window × reset cycle). The
    /// watcher polls every 5 min, so crossing detection latency is
    /// bounded by that cadence.
    ///
    /// Pre-2026-05 default was `[80, 90]` — two thresholds per
    /// window per cycle, on the theory "one early warning, one near
    /// cap." In practice that produced ~10 toasts/day worst-case
    /// per active account; users reported it as too chatty. The
    /// 90-only default trims a class of "you're approaching limits"
    /// nudges that the 80% one already conveyed, keeping only the
    /// "actually near cap" signal users acted on. Add `80` back via
    /// Settings → Notifications if you want the early warning.
    #[serde(default = "default_usage_thresholds")]
    pub notify_on_usage_thresholds: Vec<u32>,

    /// Whether the per-model 7-day sub-windows (`seven_day_opus`,
    /// `seven_day_sonnet`) participate in usage-threshold alerts.
    /// Default **false** — these sub-quotas typically track the
    /// umbrella `seven_day` window for users near cap, so leaving
    /// them on triples the 7-day alert volume for what most users
    /// experience as "one cap." The umbrella `seven_day` window is
    /// always checked regardless of this flag.
    #[serde(default)]
    pub notify_on_sub_windows: bool,

    /// Config section — per-kind "Open in…" editor preferences. Defaults
    /// to an empty `by_kind` + `fallback = "system"`, meaning the OS
    /// default handler is used until the user sets a preference. Never
    /// written to CC configuration.
    #[serde(default)]
    pub editor_defaults: EditorDefaults,

    /// Pricing tier the user is billed through (Anthropic API direct,
    /// Vertex Global / Regional, AWS Bedrock). Drives the cost-report
    /// label pill and the rate multiplier applied when rendering
    /// dollar figures. Defaults to Anthropic API — bundled rates are
    /// quoted in this tier's prices, so a fresh-install user sees the
    /// canonical numbers without a preference write. See
    /// `claudepot_core::pricing::PriceTier` for the divergence policy.
    #[serde(default)]
    pub pricing_tier: PriceTier,

    /// Network-status feature toggles. See
    /// `dev-docs/network-status.md` for the cost / cadence rationale
    /// behind each default.
    #[serde(default)]
    pub service_status: ServiceStatusPrefs,

    /// Per-category notification preferences. Populated from old
    /// scalar `notify_on_*` fields on first launch after Phase 1.5
    /// of the notification refactor (see `migrate_to_v1`). Missing
    /// entries fall back to `CategoryPrefs::default()` at read time;
    /// adding a new `Category` variant doesn't require touching the
    /// stored file.
    ///
    /// During the dual-write window (schema_version 1 → 2), this
    /// map is authoritative for emit() routing, but the old scalar
    /// fields are still kept in sync so a downgrade doesn't lose
    /// the user's toggle state.
    #[serde(default)]
    pub category_prefs: HashMap<Category, CategoryPrefs>,

    /// Schema version. Phase 1.5 migration runs when the on-disk
    /// value is < `PREFS_SCHEMA_VERSION_CURRENT`. See
    /// [`Preferences::migrate_if_needed`].
    #[serde(default)]
    pub schema_version: u32,
}

/// Toggles for the network-status feature. Field defaults are tuned
/// for "show useful information when something is wrong, stay silent
/// otherwise":
///
/// - Status-page polling defaults ON (one cheap GET every 5 min;
///   benefit of "Anthropic is degraded" awareness outweighs the
///   negligible cost).
/// - Latency probing defaults ON for window-focus only (no background
///   polling — see plan doc).
/// - OS notification defaults OFF (status-page false-positives would
///   train the user to ignore real signals).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceStatusPrefs {
    /// Background poller of `status.claude.com/api/v2/summary.json`.
    pub poll_status_page: bool,
    /// Cadence for the poller. Clamped to `[2, 60]` at consumption
    /// time so a hand-edited `preferences.json` can't DoS Anthropic's
    /// status page.
    pub poll_interval_minutes: u32,
    /// Fire an OS banner on each status transition (OK ↔ Degraded ↔
    /// Down). The bell-icon log gets the entry regardless.
    pub os_notify_on_status_change: bool,
    /// Run a HEAD-probe batch every time the window gains focus.
    pub probe_latency_on_focus: bool,
}

impl Default for ServiceStatusPrefs {
    fn default() -> Self {
        Self {
            poll_status_page: true,
            poll_interval_minutes: 5,
            os_notify_on_status_change: false,
            probe_latency_on_focus: true,
        }
    }
}

/// Helper for serde's `#[serde(default = "...")]` on a bool field.
/// Lets us default `activity_hide_thinking` to `true` without hand-
/// rolling per-field defaults — the manual `Default` impl below
/// reuses the same helper so the cold-start (no preferences.json on
/// disk) and the partial-read (file exists, field missing) paths
/// agree.
fn default_true() -> bool {
    true
}

/// Default usage-threshold list used when the field is missing in
/// the on-disk preferences file. Single near-cap threshold — see the
/// `notify_on_usage_thresholds` doc above for why this is `[90]`
/// rather than `[80, 90]`. Users can add `80` back in Settings →
/// Notifications if they want the early warning.
fn default_usage_thresholds() -> Vec<u32> {
    vec![90]
}

/// Manual `Default` so cold-start (no `preferences.json` on disk;
/// `load()` returns `Self::default()` directly) gets the same field
/// values as a partial-read (where `serde(default = "…")` per field
/// kicks in). Pre-fix, the derived `Default` set every bool to
/// `false` and every `Vec<u32>` to empty, so a fresh-install user
/// never received a "needs your answer" toast and never received a
/// usage-threshold notification — even though both are documented
/// as default-on. Reuse the helpers above so a future change to the
/// per-field defaults stays in lockstep with the cold-start defaults.
impl Default for Preferences {
    fn default() -> Self {
        Self {
            hide_dock_icon: false,
            show_window_on_startup: default_true(),
            activity_enabled: false,
            activity_consent_seen: false,
            activity_hide_thinking: default_true(),
            activity_excluded_paths: Vec::new(),
            notify_on_error: false,
            notify_on_idle_done: false,
            notify_on_stuck_minutes: None,
            notify_on_op_done: false,
            notify_on_waiting: default_true(),
            notify_on_usage_thresholds: default_usage_thresholds(),
            notify_on_sub_windows: false,
            editor_defaults: Default::default(),
            pricing_tier: PriceTier::default(),
            service_status: ServiceStatusPrefs::default(),
            category_prefs: HashMap::new(),
            schema_version: PREFS_SCHEMA_VERSION_CURRENT,
        }
    }
}

impl Preferences {
    fn path() -> PathBuf {
        paths::claudepot_data_dir().join("preferences.json")
    }

    /// Read preferences from disk, applying any pending schema
    /// migrations. The migrated form is NOT persisted automatically
    /// — the next `save()` will rewrite the file in the new shape.
    /// This makes load idempotent: repeated reads of an old file
    /// always produce the same in-memory state, with the migration
    /// landing on disk only when a setter explicitly fires.
    pub fn load() -> Self {
        let mut p = match std::fs::read_to_string(Self::path()) {
            Ok(s) => serde_json::from_str::<Self>(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        };
        p.migrate_if_needed();
        p
    }

    pub fn save(&self) -> Result<(), String> {
        let p = Self::path();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("preferences: mkdir {}: {}", parent.display(), e))?;
        }
        let s = serde_json::to_string_pretty(self)
            .map_err(|e| format!("preferences: serialize: {e}"))?;
        // Audit-fix Low #14: use the shared atomic-write helper so
        // a crash mid-write doesn't leave preferences.json
        // partially serialized and reset the user's settings on
        // next launch. Same temp-then-rename pattern every other
        // persisted file in the project uses.
        claudepot_core::fs_utils::atomic_write(&p, s.as_bytes())
            .map_err(|e| format!("preferences: atomic_write {}: {}", p.display(), e))?;
        Ok(())
    }

    /// Apply pending schema migrations in place. Idempotent — calling
    /// twice has no extra effect. Today only the 0 → 1 migration
    /// exists (scalar `notify_on_*` → `category_prefs` map).
    pub fn migrate_if_needed(&mut self) {
        if self.schema_version < 1 {
            self.migrate_to_v1();
            self.schema_version = 1;
        }
        // Future migrations (v1 → v2 to drop the scalar fields)
        // chain here.
    }

    /// 0 → 1 migration. Populates `category_prefs` from the seven
    /// scalar `notify_on_*` fields. The scalars are LEFT IN PLACE
    /// — dual-write keeps a downgrade path open until the v1 → v2
    /// migration drops them in a future release.
    fn migrate_to_v1(&mut self) {
        // Only run if the map is empty — a hand-edited
        // preferences.json could already have a populated map plus
        // a stale schema_version: 0. Respect explicit user state.
        if !self.category_prefs.is_empty() {
            return;
        }

        let mut m: HashMap<Category, CategoryPrefs> = HashMap::new();

        // Activity-related scalars → P1 categories.
        m.insert(
            Category::SessionErrorBurst,
            CategoryPrefs {
                enabled: self.notify_on_error,
                os_override: None,
            },
        );
        // Idle-done was historically conflated with op-done; both
        // map to OpDoneUnfocused. Bias toward the more
        // permissive scalar — enabled if either was on.
        m.insert(
            Category::OpDoneUnfocused,
            CategoryPrefs {
                enabled: self.notify_on_idle_done || self.notify_on_op_done,
                os_override: None,
            },
        );
        m.insert(
            Category::SessionStuck,
            CategoryPrefs {
                enabled: self.notify_on_stuck_minutes.is_some(),
                os_override: None,
            },
        );
        m.insert(
            Category::SessionWaiting,
            CategoryPrefs {
                enabled: self.notify_on_waiting,
                os_override: None,
            },
        );
        m.insert(
            Category::UsageThreshold,
            CategoryPrefs {
                enabled: !self.notify_on_usage_thresholds.is_empty(),
                os_override: None,
            },
        );

        // Service-status: legacy `os_notify_on_status_change` maps to
        // the `os_override` field — category is enabled (logs always)
        // but OS banner follows the user's preference.
        m.insert(
            Category::ServiceStatusChanged,
            CategoryPrefs {
                enabled: true,
                os_override: Some(self.service_status.os_notify_on_status_change),
            },
        );

        self.category_prefs = m;
    }

    /// Look up the effective preference for `category`. Falls back
    /// to the category-aware default (reading
    /// `display_meta().default_enabled`) when no explicit entry
    /// exists — so a fresh install with an empty map still honors
    /// per-category default policy (e.g. `SessionStuck`,
    /// `OpDoneUnfocused`, `SessionErrorBurst` default off).
    pub fn category_pref(&self, category: Category) -> CategoryPrefs {
        self.category_prefs
            .get(&category)
            .cloned()
            .unwrap_or_else(|| default_prefs_for(category))
    }

    /// Update a single category's preference and sync any legacy
    /// scalar field that mirrors it. Caller is responsible for
    /// persisting via `save()` after one or more updates.
    pub fn set_category_pref(&mut self, category: Category, prefs: CategoryPrefs) {
        self.category_prefs.insert(category, prefs.clone());
        self.sync_legacy_scalar(category, &prefs);
    }

    /// Mirror a `CategoryPrefs` update back to the legacy scalar
    /// field, if one exists for this category. Called from
    /// `set_category_pref` so the dual-write contract is enforced
    /// in one place. Categories with no legacy mirror are a no-op.
    fn sync_legacy_scalar(&mut self, category: Category, prefs: &CategoryPrefs) {
        match category {
            Category::SessionErrorBurst => self.notify_on_error = prefs.enabled,
            Category::OpDoneUnfocused => {
                self.notify_on_op_done = prefs.enabled;
                self.notify_on_idle_done = prefs.enabled;
            }
            Category::SessionStuck => {
                // Preserve the existing threshold value if any; only
                // toggle enabled-ness.
                if prefs.enabled && self.notify_on_stuck_minutes.is_none() {
                    self.notify_on_stuck_minutes = Some(15);
                } else if !prefs.enabled {
                    self.notify_on_stuck_minutes = None;
                }
            }
            Category::SessionWaiting => self.notify_on_waiting = prefs.enabled,
            Category::UsageThreshold => {
                if !prefs.enabled {
                    self.notify_on_usage_thresholds = Vec::new();
                } else if self.notify_on_usage_thresholds.is_empty() {
                    self.notify_on_usage_thresholds = vec![90];
                }
            }
            Category::ServiceStatusChanged => {
                if let Some(os) = prefs.os_override {
                    self.service_status.os_notify_on_status_change = os;
                }
            }
            // Categories with no legacy mirror: their CategoryPrefs
            // is the sole storage. Includes RotationSuggested (the
            // gap the audit flagged — now correctly user-gateable),
            // RotationApplied, RotationFailed, and the new
            // MemoryChanged / ConfigTreePatched / etc.
            _ => {}
        }
    }
}

/// Tauri-managed shared state — single mutex-guarded record.
pub struct PreferencesState(pub Mutex<Preferences>);

impl PreferencesState {
    pub fn new(p: Preferences) -> Self {
        Self(Mutex::new(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthesize a v0 preferences struct — the pre-migration shape.
    /// schema_version defaults to 0; category_prefs starts empty.
    /// All other fields take whatever scalar values the test sets.
    fn v0() -> Preferences {
        Preferences {
            schema_version: 0,
            category_prefs: HashMap::new(),
            ..Preferences::default()
        }
    }

    #[test]
    fn test_migrate_to_v1_populates_category_prefs_from_scalars() {
        let mut p = v0();
        p.notify_on_error = true;
        p.notify_on_waiting = true;
        p.notify_on_op_done = false;
        p.notify_on_idle_done = false;
        p.notify_on_usage_thresholds = vec![85, 95];
        p.service_status.os_notify_on_status_change = true;

        p.migrate_if_needed();

        assert_eq!(p.schema_version, 1);
        assert!(p.category_pref(Category::SessionErrorBurst).enabled);
        assert!(p.category_pref(Category::SessionWaiting).enabled);
        assert!(!p.category_pref(Category::OpDoneUnfocused).enabled);
        assert!(p.category_pref(Category::UsageThreshold).enabled);
        assert_eq!(
            p.category_pref(Category::ServiceStatusChanged).os_override,
            Some(true),
        );
    }

    #[test]
    fn test_migrate_is_idempotent() {
        let mut p = v0();
        p.notify_on_waiting = true;
        p.migrate_if_needed();
        let after_first = p.category_prefs.clone();
        p.migrate_if_needed();
        assert_eq!(p.category_prefs, after_first);
        assert_eq!(p.schema_version, 1);
    }

    #[test]
    fn test_migrate_respects_explicit_category_prefs() {
        // Hand-edited preferences.json could have a stale
        // schema_version: 0 but an already-populated map. Migration
        // must not overwrite that.
        let mut p = v0();
        p.category_prefs.insert(
            Category::SessionWaiting,
            CategoryPrefs {
                enabled: false,
                os_override: None,
            },
        );
        // Scalar says true — would clobber the explicit `false` if
        // the guard didn't fire.
        p.notify_on_waiting = true;
        p.migrate_if_needed();
        assert!(!p.category_pref(Category::SessionWaiting).enabled);
    }

    #[test]
    fn test_category_pref_falls_back_to_default_when_missing() {
        let mut p = v0();
        p.migrate_if_needed();
        // BannerResolved was never in any legacy scalar; the map has
        // no entry. `category_pref` must return the category-aware
        // default — BannerResolved's display_meta is enabled-by-default.
        let pr = p.category_pref(Category::BannerResolved);
        assert!(pr.enabled);
        assert!(pr.os_override.is_none());
    }

    #[test]
    fn test_category_pref_default_honors_display_meta_per_category() {
        // Audit-fix High #2: categories that ship default-OFF in
        // display_meta() (notify_on_error, notify_on_op_done,
        // notify_on_stuck_minutes families) must NOT be treated as
        // enabled when their map entry is missing. A plain
        // `CategoryPrefs::default()` would have flipped them all to
        // `true` regardless of display_meta — the bug this fix
        // closes.
        let p = Preferences {
            schema_version: PREFS_SCHEMA_VERSION_CURRENT,
            category_prefs: HashMap::new(),
            ..Preferences::default()
        };
        // SessionStuck / SessionErrorBurst / OpDoneUnfocused ship
        // default-off in display_meta (matching audit-validated
        // activity defaults).
        assert!(!p.category_pref(Category::SessionStuck).enabled);
        assert!(!p.category_pref(Category::SessionErrorBurst).enabled);
        assert!(!p.category_pref(Category::OpDoneUnfocused).enabled);
        // SessionWaiting / UsageThreshold ship default-on.
        assert!(p.category_pref(Category::SessionWaiting).enabled);
        assert!(p.category_pref(Category::UsageThreshold).enabled);
    }

    #[test]
    fn test_effective_os_surface_honors_enabled_and_override() {
        // Audit-fix High #6 helper: drives both watchers.
        let mut p = Preferences::default();
        // Disabled category yields empty surface regardless of
        // wants_os.
        p.set_category_pref(
            Category::UsageThreshold,
            CategoryPrefs {
                enabled: false,
                os_override: None,
            },
        );
        assert!(effective_os_surface(&p, Category::UsageThreshold, true).is_empty());
        // Enabled + os_override=Some(false) → no OS even if priority
        // wants it.
        p.set_category_pref(
            Category::UsageThreshold,
            CategoryPrefs {
                enabled: true,
                os_override: Some(false),
            },
        );
        assert!(effective_os_surface(&p, Category::UsageThreshold, true).is_empty());
        // Enabled + os_override=Some(true) → OS even when wants_os=false.
        p.set_category_pref(
            Category::UsageThreshold,
            CategoryPrefs {
                enabled: true,
                os_override: Some(true),
            },
        );
        assert_eq!(
            effective_os_surface(&p, Category::UsageThreshold, false),
            vec![claudepot_core::notifications::Surface::OsBanner],
        );
        // Enabled + os_override=None → follows wants_os.
        p.set_category_pref(
            Category::UsageThreshold,
            CategoryPrefs {
                enabled: true,
                os_override: None,
            },
        );
        assert_eq!(
            effective_os_surface(&p, Category::UsageThreshold, true),
            vec![claudepot_core::notifications::Surface::OsBanner],
        );
        assert!(effective_os_surface(&p, Category::UsageThreshold, false).is_empty());
    }

    #[test]
    fn test_set_category_pref_mirrors_to_legacy_scalar() {
        let mut p = Preferences {
            notify_on_waiting: true,
            ..Preferences::default()
        };
        p.set_category_pref(
            Category::SessionWaiting,
            CategoryPrefs {
                enabled: false,
                os_override: None,
            },
        );
        // Legacy scalar must follow.
        assert!(!p.notify_on_waiting);
        // And the map records the same.
        assert!(!p.category_pref(Category::SessionWaiting).enabled);
    }

    #[test]
    fn test_set_category_pref_no_legacy_mirror_is_noop_on_scalars() {
        // RotationSuggested has no scalar today; setting its pref
        // must not panic and must persist in the map.
        let mut p = Preferences::default();
        p.set_category_pref(
            Category::RotationSuggested,
            CategoryPrefs {
                enabled: false,
                os_override: None,
            },
        );
        assert!(!p.category_pref(Category::RotationSuggested).enabled);
    }

    #[test]
    fn test_set_usage_threshold_pref_preserves_existing_thresholds() {
        let mut p = Preferences {
            notify_on_usage_thresholds: vec![80, 95],
            ..Preferences::default()
        };
        // Toggling enabled off must clear the list.
        p.set_category_pref(
            Category::UsageThreshold,
            CategoryPrefs {
                enabled: false,
                os_override: None,
            },
        );
        assert!(p.notify_on_usage_thresholds.is_empty());
        // Toggling back on without explicit thresholds must
        // populate the default — empty thresholds = feature off in
        // the existing watcher, so we need at least one entry.
        p.set_category_pref(
            Category::UsageThreshold,
            CategoryPrefs {
                enabled: true,
                os_override: None,
            },
        );
        assert_eq!(p.notify_on_usage_thresholds, vec![90]);
    }

    #[test]
    fn test_serde_round_trip_with_category_prefs() {
        let mut p = Preferences::default();
        p.set_category_pref(
            Category::RotationSuggested,
            CategoryPrefs {
                enabled: false,
                os_override: Some(true),
            },
        );
        let s = serde_json::to_string(&p).unwrap();
        let back: Preferences = serde_json::from_str(&s).unwrap();
        assert_eq!(back.schema_version, PREFS_SCHEMA_VERSION_CURRENT);
        let cp = back.category_pref(Category::RotationSuggested);
        assert!(!cp.enabled);
        assert_eq!(cp.os_override, Some(true));
    }

    #[test]
    fn test_load_default_returns_current_schema_version() {
        // Cold-start (no file on disk) skips deserialization but
        // must still produce a struct at the current schema
        // version — migrate_if_needed should be a no-op in that
        // path because Default::default() sets the right version.
        let p = Preferences::default();
        assert_eq!(p.schema_version, PREFS_SCHEMA_VERSION_CURRENT);
        assert!(p.category_prefs.is_empty());
    }
}
