//! Tauri-side adapter for `LiveActivityService`.
//!
//! Implements `SessionEventListener` so the framework-free service
//! in `claudepot-core` can fan its three event types
//! (`on_aggregate`, `on_membership_changed`, `on_detail`) out to
//! the webview (`live-all` / `live::<sid>`) and the tray menu
//! rebuild without polluting the core crate with `tauri` deps.

use std::sync::Arc;

use claudepot_core::services::live_activity_service::SessionEventListener;
use claudepot_core::session_live::types::{LiveDelta, LiveSessionSummary};
use tauri::Emitter;

use crate::dto::{LiveDeltaDto, LiveSessionSummaryDto};

/// Forwards every service event to the right Tauri surface. Cheap
/// to clone — only holds an `AppHandle`. One instance is registered
/// at startup; future fan-out additions (CLI introspection, plugin
/// surfaces) can register their own listener Arc on the same
/// service.
pub struct TauriSessionEventListener {
    pub app: tauri::AppHandle,
}

impl TauriSessionEventListener {
    pub fn new(app: tauri::AppHandle) -> Arc<Self> {
        Arc::new(Self { app })
    }
}

impl SessionEventListener for TauriSessionEventListener {
    fn on_aggregate(&self, sessions: Arc<Vec<LiveSessionSummary>>) {
        let dtos: Vec<LiveSessionSummaryDto> = sessions
            .iter()
            .cloned()
            .map(LiveSessionSummaryDto::from)
            .collect();
        let _ = self.app.emit("live-all", dtos);
    }

    fn on_membership_changed(&self, _sessions: Arc<Vec<LiveSessionSummary>>) {
        // Tray rebuild is async; the listener trait is sync so we
        // hand off to the Tauri runtime. The 1s debounce is already
        // applied inside the service — every call here corresponds
        // to a real membership change post-debounce.
        let app = self.app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = crate::tray::rebuild(&app).await {
                tracing::warn!("activity tray rebuild failed: {e}");
            }
        });
    }

    fn on_detail(&self, session_id: &str, delta: LiveDelta) {
        let dto = LiveDeltaDto::from(delta);
        let _ = self.app.emit(&format!("live::{session_id}"), dto);
    }
}
