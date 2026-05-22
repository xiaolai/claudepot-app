//! Atomic load/save of `~/.claudepot/agent-events.json` — the
//! event-state ledger for `session-settled` triggers.
//!
//! `session-settled` must fire **exactly once** per (agent, session)
//! pair (PRD §7.2). Re-deriving "did this already fire?" from the
//! agent run history is fragile — a run record can be pruned by
//! `log_retention_runs` long before the session leaves the index —
//! so the fired set needs its own authoritative home.
//!
//! Mirrors `rotation::breaker_store` exactly: missing file → empty;
//! corrupt/invalid file → renamed aside to `<path>.corrupt`, return
//! empty, log a warn; a *real* I/O failure (permission denied, disk
//! gone) propagates as `Err` so the orchestrator skips the tick
//! instead of clobbering the user's real ledger on the next save.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::fs_utils::atomic_write;

/// Bumped on schema-breaking changes. A file with an unrecognized
/// version is treated as corrupt (moved aside, empty returned).
pub const SCHEMA_VERSION: u32 = 1;

/// Standard filename inside `claudepot_data_dir()`.
pub const EVENTS_FILENAME: &str = "agent-events.json";

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
    pub fn record_fire(
        &mut self,
        agent_id: &str,
        session_id: &str,
        fired_at: DateTime<Utc>,
    ) {
        if self.has_fired(agent_id, session_id) {
            return;
        }
        self.fired.push(FiredEntry {
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
            fired_at,
        });
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
            live_agent_ids.contains(&e.agent_id)
                && live_session_ids.contains(&e.session_id)
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

/// Load the ledger from the canonical path. Three-outcome contract,
/// matching `rotation::breaker_store::load`: `Ok` covers success,
/// missing file, and recovered-from-corruption; `Err` is a real I/O
/// failure.
pub fn load() -> std::io::Result<EventsFile> {
    load_from(&events_path())
}

/// Test-friendly load that takes the path directly. See [`load`].
pub fn load_from(path: &Path) -> std::io::Result<EventsFile> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(EventsFile::default());
        }
        Err(e) => return Err(e),
    };
    match serde_json::from_slice::<EventsFile>(&bytes) {
        Ok(file) => match file.validate() {
            Ok(()) => Ok(file),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "agent_events_store: parsed but invalid; moving aside and starting empty"
                );
                move_aside(path);
                Ok(EventsFile::default())
            }
        },
        Err(e) => {
            tracing::warn!(
                error = %e,
                "agent_events_store: parse failed; moving aside and starting empty"
            );
            move_aside(path);
            Ok(EventsFile::default())
        }
    }
}

fn move_aside(path: &Path) {
    let corrupt = path.with_extension("json.corrupt");
    let _ = std::fs::rename(path, corrupt);
}

/// Log + swallow real I/O errors, always returning a usable file.
/// Use only where errors cannot be propagated; new code prefers
/// [`load`].
pub fn load_or_default() -> EventsFile {
    match load() {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "agent_events_store: read failed; defaulting to empty");
            EventsFile::default()
        }
    }
}

/// Persist `file` to the canonical path. Validates before writing —
/// invalid input is rejected so on-disk files are always loadable.
pub fn save(file: &EventsFile) -> Result<(), AgentEventsError> {
    save_to(&events_path(), file)
}

/// Test-friendly save that takes the path directly.
pub fn save_to(path: &Path, file: &EventsFile) -> Result<(), AgentEventsError> {
    file.validate()?;
    let json = serde_json::to_vec_pretty(file)?;
    atomic_write(path, &json)?;
    Ok(())
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
        let live_agents: HashSet<String> =
            ["live-agent".to_string()].into_iter().collect();
        let live_sessions: HashSet<String> =
            ["live-sess".to_string()].into_iter().collect();
        let removed = f.prune(&live_agents, &live_sessions);
        assert_eq!(removed, 2);
        assert_eq!(f.fired.len(), 1);
        assert!(f.has_fired("live-agent", "live-sess"));
    }

    #[test]
    fn test_events_store_corrupt_file_is_moved_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("agent-events.json");
        std::fs::write(&p, b"this is not json").unwrap();
        let f = load_from(&p).unwrap();
        assert!(f.fired.is_empty());
        assert!(
            p.with_extension("json.corrupt").exists(),
            "corrupt file should be moved aside"
        );
    }

    #[test]
    fn test_events_store_unsupported_schema_version_is_moved_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("agent-events.json");
        std::fs::write(&p, br#"{"schema_version":99,"fired":[]}"#).unwrap();
        let f = load_from(&p).unwrap();
        assert!(f.fired.is_empty());
        assert!(p.with_extension("json.corrupt").exists());
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

    #[cfg(unix)]
    #[test]
    fn test_events_store_save_writes_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("agent-events.json");
        save_to(&p, &EventsFile::default()).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
