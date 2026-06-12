//! Atomic load/save of `~/.claudepot/agent-events.json` — the
//! event-state ledger for `session-settled` triggers.
//!
//! `session-settled` must fire **exactly once** per (agent, session)
//! pair (PRD §7.2). Re-deriving "did this already fire?" from the
//! agent run history is fragile — a run record can be pruned by
//! `log_retention_runs` long before the session leaves the index —
//! so the fired set needs its own authoritative home.
//!
//! Persistence is a thin wrapper over [`crate::json_store`] — see
//! that module for the three-outcome load contract and the
//! corruption-recovery policy (grill X23: timestamped rename-aside
//! so repeated corruption events do not overwrite the forensic copy,
//! warn on rename failure, atomic write). A *real* I/O failure
//! (permission denied, disk gone) propagates as `Err` so the
//! orchestrator skips the tick instead of clobbering the user's real
//! ledger on the next save.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::json_store::{self, SaveError};

/// Bumped on schema-breaking changes. A file with an unrecognized
/// version is treated as corrupt (moved aside, empty returned).
pub const SCHEMA_VERSION: u32 = 1;

/// Standard filename inside `claudepot_data_dir()`.
pub const EVENTS_FILENAME: &str = "agent-events.json";

/// Hard cap on ledger entries. `prune` drops pairs whose agent or
/// session is gone, but it only runs when the orchestrator ticks,
/// and a long-lived project accumulates one entry per (agent,
/// session) fire. This cap is the backstop against unbounded growth
/// (grill findings F1/F13): when it is exceeded, the oldest fires
/// (by `fired_at`) are evicted. Evicting a still-live pair lets it
/// fire once more — bounded and self-correcting (it is re-recorded
/// immediately), far cheaper than an unbounded file. Sized well
/// above any realistic agent × session fan-out.
pub const MAX_FIRED_ENTRIES: usize = 2000;

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

/// `~/.claudepot/agent-events.json` (or `$CLAUDEPOT_DATA_DIR`'d).
pub fn events_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(EVENTS_FILENAME)
}

/// One recorded fire: the `session-settled` trigger fired agent
/// `agent_id` for session `session_id` at `fired_at`. The
/// `(agent_id, session_id)` pair is the dedupe key; `fired_at` is
/// retained for forensics + a future age-based prune.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FiredEntry {
    /// The `Event`-triggered agent that fired.
    pub agent_id: String,
    /// CC/Codex `session_id` (the transcript's filename stem) of
    /// the settled session that triggered the fire.
    pub session_id: String,
    /// When the fire was recorded.
    pub fired_at: DateTime<Utc>,
}

/// Top-level on-disk shape of `~/.claudepot/agent-events.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventsFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub fired: Vec<FiredEntry>,
}

impl Default for EventsFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            fired: Vec::new(),
        }
    }
}

impl EventsFile {
    /// Validate the whole file. The store refuses to persist an
    /// invalid file, so on-disk state is always loadable + coherent.
    pub fn validate(&self) -> Result<(), AgentEventsError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(AgentEventsError::UnsupportedSchemaVersion {
                found: self.schema_version,
                expected: SCHEMA_VERSION,
            });
        }
        Ok(())
    }

    /// True iff this (agent, session) pair has already fired —
    /// the fire-once guard.
    pub fn has_fired(&self, agent_id: &str, session_id: &str) -> bool {
        self.fired
            .iter()
            .any(|e| e.agent_id == agent_id && e.session_id == session_id)
    }

    /// Record a fire. Idempotent — a pair already present is not
    /// duplicated, so a double `record_fire` cannot inflate the
    /// ledger.
    pub fn record_fire(&mut self, agent_id: &str, session_id: &str, fired_at: DateTime<Utc>) {
        if self.has_fired(agent_id, session_id) {
            return;
        }
        self.fired.push(FiredEntry {
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
            fired_at,
        });
        // Backstop against unbounded growth: keep only the newest
        // MAX_FIRED_ENTRIES by `fired_at`.
        if self.fired.len() > MAX_FIRED_ENTRIES {
            self.fired.sort_by_key(|e| e.fired_at);
            let overflow = self.fired.len() - MAX_FIRED_ENTRIES;
            // grill X12: an eviction here is silent-by-default — the
            // evicted (agent, session) pair will re-fire (and re-bill)
            // the next time it shows up in the live session index. A
            // user investigating "why did this agent run twice on the
            // same session?" had no breadcrumb. Log loudly per batch
            // so the trade-off is visible. We deliberately don't emit
            // a one-shot notification: adding a category here requires
            // the full lockstep change (Rust enum + priority +
            // display_meta + all() + EXPECTED counter + TS Category
            // union + CATEGORY_NAMES + priorityForCategory + prefs.ts
            // + fixture regen). The log path is sufficient for the
            // current operational shape; the notification can be
            // promoted later if eviction starts firing in the wild.
            let oldest = self.fired.first().map(|e| e.fired_at);
            tracing::warn!(
                evicted = overflow,
                cap = MAX_FIRED_ENTRIES,
                oldest_fired_at = ?oldest,
                "agent_events_store: ledger cap reached; oldest pairs \
                 evicted — they may re-fire (and re-bill) if still in \
                 the live session index"
            );
            self.fired.drain(0..overflow);
        }
    }

    /// Undo a `record_fire` that was never persisted (grill finding
    /// X4 — the F1 prune-save hole). The orchestrator's dispatch
    /// loop calls `record_fire` and then `save`; if the save fails
    /// the in-memory mutation is left behind, and the *post-loop*
    /// prune save then flushes it to disk — the pair shows as fired
    /// without ever running.
    ///
    /// Returns `true` iff a matching entry was found and removed
    /// (so the caller can assert "the in-memory ledger is clean").
    /// Idempotent: a second call on a now-clean ledger returns
    /// `false`.
    pub fn unrecord_fire(&mut self, agent_id: &str, session_id: &str) -> bool {
        let before = self.fired.len();
        self.fired
            .retain(|e| !(e.agent_id == agent_id && e.session_id == session_id));
        before != self.fired.len()
    }

    /// Drop ledger entries whose agent is no longer installed OR
    /// whose session no longer exists in the index. Keeps the file
    /// from growing an unbounded set of stale pairs as agents and
    /// transcripts are deleted (PRD §7.2 — "Prune entries for
    /// sessions/agents that no longer exist"). Returns the number
    /// of entries removed.
    pub fn prune(
        &mut self,
        live_agent_ids: &HashSet<String>,
        live_session_ids: &HashSet<String>,
    ) -> usize {
        let before = self.fired.len();
        self.fired.retain(|e| {
            live_agent_ids.contains(&e.agent_id) && live_session_ids.contains(&e.session_id)
        });
        before - self.fired.len()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentEventsError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("schema version {found} is unsupported (expected {expected})")]
    UnsupportedSchemaVersion { found: u32, expected: u32 },
}

/// Store name used in log messages.
const STORE: &str = "agent_events_store";

impl json_store::Validate for EventsFile {
    type Error = AgentEventsError;
    fn validate(&self) -> Result<(), AgentEventsError> {
        // Delegates to the inherent method (inherent methods win
        // resolution over trait methods, so this is not recursion).
        EventsFile::validate(self)
    }
}

/// Load the ledger from the canonical path under the three-outcome
/// contract (see [`crate::json_store`]): `Ok` covers success,
/// missing file, and recovered-from-corruption; `Err` is a real I/O
/// failure.
pub fn load() -> std::io::Result<EventsFile> {
    load_from(&events_path())
}

/// Test-friendly load that takes the path directly. See [`load`].
pub fn load_from(path: &Path) -> std::io::Result<EventsFile> {
    json_store::load(path, STORE)
}

/// Log + swallow real I/O errors, always returning a usable file.
/// Use only where errors cannot be propagated; new code prefers
/// [`load`].
pub fn load_or_default() -> EventsFile {
    json_store::load_or_default(&events_path(), STORE)
}

/// Persist `file` to the canonical path. Validates before writing —
/// invalid input is rejected so on-disk files are always loadable.
pub fn save(file: &EventsFile) -> Result<(), AgentEventsError> {
    save_to(&events_path(), file)
}

/// Test-friendly save that takes the path directly.
pub fn save_to(path: &Path, file: &EventsFile) -> Result<(), AgentEventsError> {
    json_store::save(path, file).map_err(|e| match e {
        SaveError::Validation(v) => v,
        SaveError::Serde(s) => s.into(),
        SaveError::Io(io) => io.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 22, 12, 0, 0).unwrap()
    }

    #[test]
    fn test_events_store_load_missing_file_yields_default() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("nope.json");
        let f = load_from(&p).unwrap();
        assert_eq!(f.schema_version, SCHEMA_VERSION);
        assert!(f.fired.is_empty());
    }

    #[test]
    fn test_events_store_save_then_load_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("agent-events.json");
        let mut file = EventsFile::default();
        file.record_fire("agent-a", "sess-1", ts());
        save_to(&p, &file).unwrap();
        let back = load_from(&p).unwrap();
        assert_eq!(back, file);
    }

    #[test]
    fn test_events_store_record_fire_is_idempotent() {
        let mut f = EventsFile::default();
        f.record_fire("a", "s", ts());
        f.record_fire("a", "s", ts());
        assert_eq!(f.fired.len(), 1, "a repeated pair must not duplicate");
        assert!(f.has_fired("a", "s"));
        assert!(!f.has_fired("a", "other"));
        assert!(!f.has_fired("other", "s"));
    }

    #[test]
    fn test_events_store_prune_drops_dead_agents_and_sessions() {
        let mut f = EventsFile::default();
        f.record_fire("live-agent", "live-sess", ts());
        f.record_fire("dead-agent", "live-sess", ts());
        f.record_fire("live-agent", "dead-sess", ts());
        let live_agents: HashSet<String> = ["live-agent".to_string()].into_iter().collect();
        let live_sessions: HashSet<String> = ["live-sess".to_string()].into_iter().collect();
        let removed = f.prune(&live_agents, &live_sessions);
        assert_eq!(removed, 2);
        assert_eq!(f.fired.len(), 1);
        assert!(f.has_fired("live-agent", "live-sess"));
    }

    // Generic store behaviors (corrupt-rename-aside under a
    // timestamped name, the X23 repeated-corruption forensics,
    // permission-denied propagation, 0600 writes) are covered once
    // in `crate::json_store::tests`; the tests here exercise the
    // ledger-specific schema + dedupe logic and the store wiring.

    #[test]
    fn test_events_store_unsupported_schema_version_is_moved_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("agent-events.json");
        std::fs::write(&p, br#"{"schema_version":99,"fired":[]}"#).unwrap();
        let f = load_from(&p).unwrap();
        assert!(f.fired.is_empty());
        let copies = crate::json_store::corrupt_siblings(&p);
        assert_eq!(copies.len(), 1);
    }

    #[test]
    fn test_events_store_schema_version_defaults_when_omitted() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("agent-events.json");
        std::fs::write(&p, br#"{"fired":[]}"#).unwrap();
        let f = load_from(&p).unwrap();
        assert_eq!(f.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn test_events_store_save_rejects_invalid_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("agent-events.json");
        let bad = EventsFile {
            schema_version: 99,
            fired: Vec::new(),
        };
        let err = save_to(&p, &bad);
        assert!(matches!(
            err,
            Err(AgentEventsError::UnsupportedSchemaVersion { .. })
        ));
        assert!(!p.exists(), "rejected file must never be written");
    }

    #[test]
    fn test_events_store_unrecord_fire_removes_in_memory_entry() {
        // X4: when the orchestrator records a fire and the
        // post-record save fails, the in-memory mutation must be
        // undoable so the post-loop prune-save does not flush it.
        let mut f = EventsFile::default();
        f.record_fire("a", "s1", ts());
        f.record_fire("a", "s2", ts());
        assert!(f.unrecord_fire("a", "s1"), "the (a, s1) pair was present");
        assert!(!f.has_fired("a", "s1"), "the pair must be gone");
        assert!(f.has_fired("a", "s2"), "other pairs are untouched");
        assert!(
            !f.unrecord_fire("a", "s1"),
            "a second unrecord on the same pair returns false"
        );
    }

    #[test]
    fn test_events_store_unrecord_fire_unknown_pair_is_noop() {
        let mut f = EventsFile::default();
        f.record_fire("a", "s1", ts());
        assert!(!f.unrecord_fire("nobody", "nothing"));
        assert_eq!(f.fired.len(), 1, "ledger unchanged when the pair is absent");
    }

    #[test]
    fn test_events_store_record_fire_caps_ledger_size() {
        let mut f = EventsFile::default();
        let base = ts();
        for i in 0..(MAX_FIRED_ENTRIES + 50) {
            f.record_fire(
                "agent",
                &format!("sess-{i}"),
                base + chrono::Duration::seconds(i as i64),
            );
        }
        assert_eq!(
            f.fired.len(),
            MAX_FIRED_ENTRIES,
            "the ledger is capped at MAX_FIRED_ENTRIES"
        );
        // The oldest entries were evicted; the newest survive.
        assert!(!f.has_fired("agent", "sess-0"), "oldest entry evicted");
        assert!(
            f.has_fired("agent", &format!("sess-{}", MAX_FIRED_ENTRIES + 49)),
            "newest entry kept"
        );
    }
}
