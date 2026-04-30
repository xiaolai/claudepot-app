//! Persisted UI preferences — a tiny JSON file in the Claudepot data dir.
//!
//! Read synchronously from `setup()` before the webview window is shown
//! so `hide_dock_icon` can flip the activation policy early enough to
//! avoid a visible dock-icon flash on cold launch. Any preference that
//! the CLI doesn't care about belongs here, not in `claudepot-core`.

use claudepot_core::config_view::model::EditorDefaults;
use claudepot_core::paths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

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
    /// crosses them. Empty vec = feature off; the default `[80, 90]`
    /// gives one early-warning + one near-cap nudge per (window ×
    /// reset cycle). The watcher polls every 5 min, so crossing
    /// detection latency is bounded by that cadence.
    #[serde(default = "default_usage_thresholds")]
    pub notify_on_usage_thresholds: Vec<u32>,

    /// Config section — per-kind "Open in…" editor preferences. Defaults
    /// to an empty `by_kind` + `fallback = "system"`, meaning the OS
    /// default handler is used until the user sets a preference. Never
    /// written to CC configuration.
    #[serde(default)]
    pub editor_defaults: EditorDefaults,
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
/// the on-disk preferences file. Picked to give one early warning
/// (80%) and one near-cap nudge (90%) per cycle without being
/// chatty. Users can edit the list in Settings → Notifications.
fn default_usage_thresholds() -> Vec<u32> {
    vec![80, 90]
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
            editor_defaults: Default::default(),
        }
    }
}

impl Preferences {
    fn path() -> PathBuf {
        paths::claudepot_data_dir().join("preferences.json")
    }

    pub fn load() -> Self {
        match std::fs::read_to_string(Self::path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let p = Self::path();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("preferences: mkdir {}: {}", parent.display(), e))?;
        }
        let s = serde_json::to_string_pretty(self)
            .map_err(|e| format!("preferences: serialize: {e}"))?;
        std::fs::write(&p, s).map_err(|e| format!("preferences: write {}: {}", p.display(), e))?;
        Ok(())
    }
}

/// Tauri-managed shared state — single mutex-guarded record.
pub struct PreferencesState(pub Mutex<Preferences>);

impl PreferencesState {
    pub fn new(p: Preferences) -> Self {
        Self(Mutex::new(p))
    }
}
