//! Per-device read state — what powers the unread badges.
//!
//! Keyed by `(device_id, session_id)`, because two phones must not clear
//! each other's badges. The value is a **count of events consumed**, not
//! a timestamp and not the index of the last one: it is the same number
//! the list already reports as `event_count`, so "unread" is a
//! subtraction rather than a second traversal of the transcript.
//!
//! Count and last-index differ by one, and the field was originally
//! *named* for the index while every caller stored the count. Nothing
//! misbehaved, because the subtraction happened to match — but a caller
//! following the field's own documentation would have left one event
//! unread forever. The name now matches what is stored.
//!
//! ## Why this recovers silently where its neighbours fail loud
//!
//! `remote-devices.json` is the revocation list and `remote-config.json`
//! holds the login throttle; a silent reset of either hands something
//! back to an attacker, so both fail loud. This file is a badge cache.
//! Losing it clears every badge, which is exactly what tapping through
//! the list would have done, and nothing about the machine's security
//! depends on it. It takes `json_store`'s ordinary
//! move-aside-and-start-empty contract.
//!
//! ## Why an index and not a timestamp
//!
//! A transcript's `last_ts` comes from CC's own clock and a phone's
//! clock is not that clock. Comparing them would make the badge depend
//! on clock skew across two devices. A count is monotonic within a file
//! and needs no shared clock. It moves backwards only when the index is
//! rebuilt or `session slim` rewrites a transcript smaller — both
//! handled by saturating at zero, which reads as "all read".
//!
//! ## One writer at a time
//!
//! `mark_at` is a read-modify-write over a whole file, so two marks
//! landing together would both read the old bytes and the later write
//! would discard the earlier one. Atomic rename gives crash-safety, not
//! concurrency-safety — the same distinction `settings_mutex` exists
//! for. A process-local mutex serializes the writers this process has,
//! which is all of them: nothing outside Claudepot writes this file.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::json_store::{self, SaveError, Validate};

pub const READ_STATE_FILENAME: &str = "remote-read-state.json";

const STORE: &str = "remote_read_state";

pub const SCHEMA_VERSION: u32 = 1;

/// Sessions remembered per device.
///
/// The list endpoint serves at most `DEFAULT_HISTORY` finished sessions
/// plus the live ones, so anything beyond a few dozen can never be shown
/// again. The cap keeps a file that is written on every thread open from
/// growing without bound on a machine with thousands of sessions.
const MAX_SESSIONS_PER_DEVICE: usize = 200;

/// Devices remembered. Revoking a device does not prune this file — the
/// cap is what stops a long-lived install accumulating dead entries.
const MAX_DEVICES: usize = 32;

/// Ceiling on a single read mark.
///
/// The count arrives from the client, and the mark only ever moves
/// forward — so one absurd value (`usize::MAX`, a units mix-up, a
/// serialisation bug) permanently suppresses that device's badges for
/// that session with nothing on screen saying why. Forward-only is the
/// right rule and it is exactly what makes a bad value unrecoverable.
///
/// The server does **not** resolve the session's true event total to
/// clamp against: that costs a full transcript read on every mark, on a
/// path a phone hits per session per open. A fixed ceiling is the cheap
/// half of the guard — it cannot catch a wrong-but-plausible count, and
/// does not try to. It catches the unbounded ones, and it fails loud so
/// a client bug surfaces as an error instead of as missing badges.
///
/// 10 million events is far past any real transcript (the largest here
/// is ~14 MB / tens of thousands of events) and far below `usize::MAX`.
pub const MAX_THROUGH_COUNT: usize = 10_000_000;

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SeenMark {
    /// Events this device has consumed — a **count**, matching
    /// `SessionRow::event_count`, not the index of the last one.
    pub through_count: usize,
    /// When it looked. The pruning order — without it, dropping an
    /// entry at the cap would be arbitrary.
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceEntry {
    #[serde(default)]
    pub sessions: BTreeMap<String, SeenMark>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadStateFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub devices: BTreeMap<String, DeviceEntry>,
}

impl Default for ReadStateFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            devices: BTreeMap::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("read-state file is schema version {found}, expected {expected}")]
    UnknownSchema { expected: u32, found: u32 },
}

impl Validate for ReadStateFile {
    type Error = ValidationError;
    fn validate(&self) -> Result<(), Self::Error> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::UnknownSchema {
                expected: SCHEMA_VERSION,
                found: self.schema_version,
            });
        }
        Ok(())
    }
}

pub fn read_state_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(READ_STATE_FILENAME)
}

/// One device's view, resolved once per list request.
#[derive(Debug, Clone, Default)]
pub struct DeviceReadState {
    marks: BTreeMap<String, usize>,
}

impl DeviceReadState {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Events this device has consumed for `session_id`, or `None` when
    /// it has never opened it.
    ///
    /// The distinction is the whole badge policy: "never looked" has no
    /// baseline to count against, and collapsing it to zero would put
    /// the entire event total on every row of a fresh device.
    pub fn seen(&self, session_id: &str) -> Option<usize> {
        self.marks.get(session_id).copied()
    }

    /// Test seam — production reads come from disk.
    pub fn set(&mut self, session_id: &str, through_count: usize) {
        self.marks.insert(session_id.to_string(), through_count);
    }
}

/// Read state for one device.
///
/// An unknown device — or none, because the caller had no device id —
/// has no marks, so every session reads as "no baseline" and carries no
/// badge. That is the deliberate first-run state: see
/// [`DeviceReadState::seen`].
pub fn load_for(device_id: Option<Uuid>) -> DeviceReadState {
    let Some(id) = device_id else {
        return DeviceReadState::empty();
    };
    load_for_at(&read_state_path(), id)
}

pub fn load_for_at(path: &std::path::Path, device_id: Uuid) -> DeviceReadState {
    let file: ReadStateFile = json_store::load_or_default(path, STORE);
    let Some(entry) = file.devices.get(&device_id.to_string()) else {
        return DeviceReadState::empty();
    };
    DeviceReadState {
        marks: entry
            .sessions
            .iter()
            .map(|(k, v)| (k.clone(), v.through_count))
            .collect(),
    }
}

/// Record that `device_id` has consumed `through_count` events of
/// `session_id`.
///
/// The mark only ever moves forward. A client that scrolls up and
/// re-reports a lower count must not resurrect badges it already
/// cleared.
pub fn mark(device_id: Uuid, session_id: &str, through_count: usize) -> Result<usize, MarkError> {
    mark_at(
        &read_state_path(),
        device_id,
        session_id,
        through_count,
        Utc::now(),
    )
}

/// Serializes the read-modify-write. See the module docs — atomic
/// rename is crash-safety, not concurrency-safety.
static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Two failures are reachable, and neither is a read.
///
/// The read side goes through `json_store::load_or_default`, which
/// recovers from every I/O and parse failure by starting empty — the
/// right contract for a badge cache, and the reason there is no `Read`
/// variant here. A variant that cannot be constructed is a claim about
/// failure modes that is not true.
#[derive(Debug, thiserror::Error)]
pub enum MarkError {
    #[error("cannot write the read-state file")]
    Write,
    #[error("read mark {got} is above the {max} ceiling")]
    Implausible { got: usize, max: usize },
}

pub fn mark_at(
    path: &std::path::Path,
    device_id: Uuid,
    session_id: &str,
    through_count: usize,
    now: DateTime<Utc>,
) -> Result<usize, MarkError> {
    // Held across the whole read-modify-write, not just the write. A
    // poisoned lock is recovered rather than propagated: the data it
    // guards is a badge cache, and refusing to record a read because
    // some other thread panicked would be a worse outcome than the
    // interleaving this guards against.
    if through_count > MAX_THROUGH_COUNT {
        return Err(MarkError::Implausible {
            got: through_count,
            max: MAX_THROUGH_COUNT,
        });
    }

    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut file: ReadStateFile = json_store::load_or_default(path, STORE);
    let entry = file.devices.entry(device_id.to_string()).or_default();

    let mark = entry
        .sessions
        .entry(session_id.to_string())
        .or_insert(SeenMark {
            through_count: 0,
            at: now,
        });
    mark.through_count = mark.through_count.max(through_count);
    mark.at = now;
    let settled = mark.through_count;

    prune(&mut file);

    json_store::save(path, &file).map_err(|e| {
        match e {
            SaveError::Io(io) => tracing::warn!(error = %io, "read-state: write failed"),
            SaveError::Serde(s) => tracing::warn!(error = %s, "read-state: serialize failed"),
            SaveError::Validation(v) => tracing::warn!(error = %v, "read-state: refused own value"),
        }
        MarkError::Write
    })?;
    Ok(settled)
}

/// Drop the least recently touched entries once either cap is hit.
fn prune(file: &mut ReadStateFile) {
    for entry in file.devices.values_mut() {
        if entry.sessions.len() <= MAX_SESSIONS_PER_DEVICE {
            continue;
        }
        let mut by_age: Vec<(String, DateTime<Utc>)> = entry
            .sessions
            .iter()
            .map(|(k, v)| (k.clone(), v.at))
            .collect();
        by_age.sort_by(|a, b| b.1.cmp(&a.1));
        let keep: std::collections::HashSet<String> = by_age
            .into_iter()
            .take(MAX_SESSIONS_PER_DEVICE)
            .map(|(k, _)| k)
            .collect();
        entry.sessions.retain(|k, _| keep.contains(k));
    }

    if file.devices.len() > MAX_DEVICES {
        let mut by_age: Vec<(String, DateTime<Utc>)> = file
            .devices
            .iter()
            .map(|(k, v)| {
                let newest = v
                    .sessions
                    .values()
                    .map(|m| m.at)
                    .max()
                    .unwrap_or(DateTime::<Utc>::MIN_UTC);
                (k.clone(), newest)
            })
            .collect();
        by_age.sort_by(|a, b| b.1.cmp(&a.1));
        let keep: std::collections::HashSet<String> = by_age
            .into_iter()
            .take(MAX_DEVICES)
            .map(|(k, _)| k)
            .collect();
        file.devices.retain(|k, _| keep.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(mins: i64) -> DateTime<Utc> {
        DateTime::<Utc>::MIN_UTC
            + chrono::Duration::days(365 * 2000)
            + chrono::Duration::minutes(mins)
    }

    #[test]
    fn an_unknown_device_has_seen_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join(READ_STATE_FILENAME);
        let st = load_for_at(&p, Uuid::new_v4());
        assert_eq!(st.seen("anything"), None);
    }

    #[test]
    fn a_mark_round_trips_for_the_device_that_made_it() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join(READ_STATE_FILENAME);
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        mark_at(&p, a, "s1", 40, t(0)).unwrap();

        assert_eq!(load_for_at(&p, a).seen("s1"), Some(40));
        assert_eq!(
            load_for_at(&p, b).seen("s1"),
            None,
            "one phone must not clear another's badges"
        );
    }

    #[test]
    fn a_mark_only_moves_forward() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join(READ_STATE_FILENAME);
        let d = Uuid::new_v4();
        mark_at(&p, d, "s1", 40, t(0)).unwrap();
        let settled = mark_at(&p, d, "s1", 10, t(1)).unwrap();
        assert_eq!(settled, 40, "scrolling up must not resurrect badges");
        assert_eq!(load_for_at(&p, d).seen("s1"), Some(40));
    }

    #[test]
    fn a_corrupt_file_starts_empty_rather_than_failing() {
        // This is a badge cache. Losing it marks everything read, which
        // is what tapping through the list would have done anyway.
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join(READ_STATE_FILENAME);
        std::fs::write(&p, "{ not json").unwrap();
        let st = load_for_at(&p, Uuid::new_v4());
        assert_eq!(st.seen("s1"), None);
        assert!(
            !json_store::corrupt_siblings(&p).is_empty(),
            "the bad file must be kept, not deleted"
        );
    }

    #[test]
    fn sessions_are_capped_per_device_keeping_the_newest() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join(READ_STATE_FILENAME);
        let d = Uuid::new_v4();
        for i in 0..(MAX_SESSIONS_PER_DEVICE + 25) {
            mark_at(&p, d, &format!("s{i:04}"), i, t(i as i64)).unwrap();
        }
        let file: ReadStateFile = json_store::load_or_default(&p, STORE);
        let entry = &file.devices[&d.to_string()];
        assert_eq!(entry.sessions.len(), MAX_SESSIONS_PER_DEVICE);
        let newest = format!("s{:04}", MAX_SESSIONS_PER_DEVICE + 24);
        assert!(
            entry.sessions.contains_key(&newest),
            "the newest mark was pruned"
        );
        assert!(
            !entry.sessions.contains_key("s0000"),
            "the oldest mark survived"
        );
    }

    #[test]
    fn devices_are_capped_keeping_the_most_recently_active() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join(READ_STATE_FILENAME);
        let mut ids = Vec::new();
        for i in 0..(MAX_DEVICES + 5) {
            let d = Uuid::new_v4();
            ids.push(d);
            mark_at(&p, d, "s1", 1, t(i as i64)).unwrap();
        }
        let file: ReadStateFile = json_store::load_or_default(&p, STORE);
        assert_eq!(file.devices.len(), MAX_DEVICES);
        assert!(file.devices.contains_key(&ids.last().unwrap().to_string()));
        assert!(!file.devices.contains_key(&ids[0].to_string()));
    }

    #[test]
    fn concurrent_marks_for_different_sessions_do_not_drop_each_other() {
        // Without the write lock this fails: every thread reads the same
        // bytes, adds its own session, and the last rename wins — so one
        // mark survives out of eight and seven badges stay lit.
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join(READ_STATE_FILENAME);
        let d = Uuid::new_v4();

        std::thread::scope(|scope| {
            for i in 0..8 {
                let p = p.clone();
                scope.spawn(move || {
                    mark_at(&p, d, &format!("s{i}"), i + 1, t(i as i64)).unwrap();
                });
            }
        });

        let st = load_for_at(&p, d);
        for i in 0..8 {
            assert_eq!(
                st.seen(&format!("s{i}")),
                Some(i + 1),
                "mark for s{i} was lost to a concurrent write"
            );
        }
    }

    #[test]
    fn a_future_schema_is_treated_as_unreadable() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join(READ_STATE_FILENAME);
        std::fs::write(&p, r#"{"schema_version": 99, "devices": {}}"#).unwrap();
        let st = load_for_at(&p, Uuid::new_v4());
        assert_eq!(st.seen("s1"), None);
    }

    #[test]
    fn an_implausible_mark_is_refused_rather_than_stored() {
        // The mark only moves forward, so one absurd value would
        // suppress this device's badges for this session permanently —
        // and silently, which is the part that makes it a bug rather
        // than a nuisance.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rs.json");
        let dev = Uuid::new_v4();

        assert!(matches!(
            mark_at(&path, dev, "sess", usize::MAX, Utc::now()),
            Err(MarkError::Implausible { .. })
        ));
        assert!(matches!(
            mark_at(&path, dev, "sess", MAX_THROUGH_COUNT + 1, Utc::now()),
            Err(MarkError::Implausible { .. })
        ));

        // Nothing was recorded, so a later honest mark still works.
        assert_eq!(mark_at(&path, dev, "sess", 12, Utc::now()).unwrap(), 12);
        // And the ceiling itself is allowed.
        assert_eq!(
            mark_at(&path, dev, "sess", MAX_THROUGH_COUNT, Utc::now()).unwrap(),
            MAX_THROUGH_COUNT
        );
    }
}
