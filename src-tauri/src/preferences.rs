//! Persisted UI preferences — a tiny JSON file in the Claudepot data dir.
//!
//! Read synchronously from `setup()` before the webview window is shown
//! so `hide_dock_icon` can flip the activation policy early enough to
//! avoid a visible dock-icon flash on cold launch. Any preference that
//! the CLI doesn't care about belongs here, not in `claudepot-core`.

use claudepot_core::paths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Preferences {
    /// macOS-only. When true, the app runs as an accessory: no dock
    /// icon, no Cmd+Tab entry, no application menu bar. Tray-only.
    pub hide_dock_icon: bool,

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

    /// Per-trigger notification toggles, all default off. M5 wires
    /// these to the tauri-plugin-notification backend.
    pub notify_on_error: bool,
    pub notify_on_idle_done: bool,
    /// None = feature off; Some(N) = fire after N minutes stuck.
    pub notify_on_stuck_minutes: Option<u32>,
    /// None = feature off; Some(x) = fire when session spend ≥ $x.
    /// Populated once the pricing module lands in M4.
    pub notify_on_spend_usd: Option<f32>,
}

/// Helper for serde's `#[serde(default = "...")]` on a bool field.
/// Lets us default `activity_hide_thinking` to `true` without hand-
/// rolling a `Default` impl for the whole struct.
fn default_true() -> bool {
    true
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
        std::fs::write(&p, s)
            .map_err(|e| format!("preferences: write {}: {}", p.display(), e))?;
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
