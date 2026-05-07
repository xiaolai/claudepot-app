//! `tipsHistory` reader and Claudepot snapshot ledger.
//!
//! `~/.claude.json` carries `tipsHistory: { id → numStartups }` and
//! `numStartups: number`. Both are integers — no timestamps. We
//! convert to wall-clock time via the snapshot mechanism described
//! in `dev-docs/cc-tips-ledger.md` §6.
//!
//! Snapshot file: `~/.claudepot/cc_tips_snapshots.jsonl`
//! - Append-only.
//! - One JSON object per line: `{ts, numStartups, tipsHistory}`.
//! - FIFO-capped at `SNAPSHOT_CAP` entries.
//! - Hourly debounce: writes are skipped if the most recent
//!   snapshot is < 1 h old.
//! - Corrupt file → rotate to `.corrupt` and start fresh.

use crate::cc_tips::error::{TipsError, TipsResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const SNAPSHOT_CAP: usize = 200;
pub const SNAPSHOT_DEBOUNCE_MS: i64 = 60 * 60 * 1000; // 1 h

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub ts: i64,
    pub num_startups: u32,
    pub tips_history: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastSeen {
    /// "3 days ago" / "yesterday" / "today" / "just now"
    pub relative: String,
    pub startup_count_when_seen: u32,
    /// True if no snapshot covers the matching count (tip was shown
    /// before Claudepot started taking snapshots).
    pub exact_unknown: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TipsHistoryRead {
    pub num_startups: u32,
    pub tips_history: BTreeMap<String, u32>,
}

pub fn read_tips_history(global_config_path: Option<PathBuf>) -> TipsResult<TipsHistoryRead> {
    let path = global_config_path.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/"))
            .join(".claude.json")
    });
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(TipsHistoryRead {
                num_startups: 0,
                tips_history: BTreeMap::new(),
            });
        }
        Err(source) => {
            return Err(TipsError::ConfigRead {
                path: path.to_string_lossy().into_owned(),
                source,
            });
        }
    };
    let v: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|source| TipsError::ConfigParse {
            path: path.to_string_lossy().into_owned(),
            source,
        })?;
    let num_startups = v
        .get("numStartups")
        .and_then(|x| x.as_u64())
        .unwrap_or(0) as u32;
    let mut tips_history: BTreeMap<String, u32> = BTreeMap::new();
    if let Some(map) = v.get("tipsHistory").and_then(|x| x.as_object()) {
        for (k, val) in map {
            if let Some(n) = val.as_u64() {
                tips_history.insert(k.clone(), n as u32);
            }
        }
    }
    Ok(TipsHistoryRead {
        num_startups,
        tips_history,
    })
}

pub struct SnapshotLog {
    path: PathBuf,
}

impl SnapshotLog {
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn default_path() -> TipsResult<PathBuf> {
        let home = dirs::home_dir().ok_or(TipsError::NoHome)?;
        Ok(home.join(".claudepot").join("cc_tips_snapshots.jsonl"))
    }

    pub fn read_all(&self) -> TipsResult<Vec<Snapshot>> {
        let bytes = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(TipsError::SnapshotRead {
                    path: self.path.to_string_lossy().into_owned(),
                    source,
                });
            }
        };
        let mut out = Vec::new();
        let mut had_error = false;
        for line in bytes.split(|b| *b == b'\n') {
            if line.is_empty() {
                continue;
            }
            match serde_json::from_slice::<Snapshot>(line) {
                Ok(s) => out.push(s),
                Err(_) => {
                    had_error = true;
                }
            }
        }
        if had_error && out.is_empty() {
            // Rotate corrupt file aside; start fresh.
            let corrupt = self.path.with_extension("jsonl.corrupt");
            let _ = std::fs::rename(&self.path, &corrupt);
        }
        Ok(out)
    }

    /// Append a snapshot if (a) snapshots are missing or (b) the
    /// most recent is older than `SNAPSHOT_DEBOUNCE_MS`. Returns
    /// `Ok(true)` if a snapshot was written.
    pub fn record_if_due(&self, now_ms: i64, snapshot: Snapshot) -> TipsResult<bool> {
        let entries = self.read_all()?;
        if let Some(last) = entries.last() {
            if (now_ms - last.ts).abs() < SNAPSHOT_DEBOUNCE_MS {
                return Ok(false);
            }
        }
        // FIFO trim. Read+rewrite is fine at 200-entry cap.
        let mut keep = entries;
        keep.push(snapshot);
        if keep.len() > SNAPSHOT_CAP {
            let drop = keep.len() - SNAPSHOT_CAP;
            keep.drain(..drop);
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| TipsError::SnapshotWrite {
                path: self.path.to_string_lossy().into_owned(),
                source,
            })?;
        }
        let mut buf = String::new();
        for s in &keep {
            buf.push_str(
                &serde_json::to_string(s).map_err(|e| TipsError::SnapshotWrite {
                    path: self.path.to_string_lossy().into_owned(),
                    source: std::io::Error::other(e),
                })?,
            );
            buf.push('\n');
        }
        std::fs::write(&self.path, buf).map_err(|source| TipsError::SnapshotWrite {
            path: self.path.to_string_lossy().into_owned(),
            source,
        })?;
        Ok(true)
    }

    /// Resolve the wall-clock "last seen" for one tip. Algorithm:
    /// find the **oldest** snapshot S where `S.tips_history[id] ==
    /// startup_count`. That snapshot is the first observation
    /// after the tip was last shown, so its timestamp is our anchor.
    pub fn resolve_last_seen(
        &self,
        id: &str,
        startup_count: u32,
        now_ms: i64,
    ) -> TipsResult<Option<LastSeen>> {
        let snaps = self.read_all()?;
        for s in &snaps {
            if let Some(n) = s.tips_history.get(id) {
                if *n == startup_count {
                    return Ok(Some(LastSeen {
                        relative: humanize_relative(now_ms - s.ts),
                        startup_count_when_seen: startup_count,
                        exact_unknown: false,
                    }));
                }
            }
        }
        // No snapshot covers this exact count: tip was shown before
        // we started observing. Fall back to "exact unknown".
        Ok(Some(LastSeen {
            relative: "exact time unknown".to_string(),
            startup_count_when_seen: startup_count,
            exact_unknown: true,
        }))
    }
}

fn humanize_relative(diff_ms: i64) -> String {
    let abs = diff_ms.abs();
    let sec = abs / 1000;
    let min = sec / 60;
    let hr = min / 60;
    let day = hr / 24;
    if day >= 30 {
        let m = day / 30;
        if m == 1 {
            "1 month ago".to_string()
        } else {
            format!("{m} months ago")
        }
    } else if day >= 2 {
        format!("{day} days ago")
    } else if day == 1 {
        "yesterday".to_string()
    } else if hr >= 1 {
        if hr == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{hr} hours ago")
        }
    } else if min >= 1 {
        if min == 1 {
            "1 minute ago".to_string()
        } else {
            format!("{min} minutes ago")
        }
    } else {
        "just now".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(ts: i64, n: u32, kv: &[(&str, u32)]) -> Snapshot {
        let mut th = BTreeMap::new();
        for (k, v) in kv {
            th.insert((*k).to_string(), *v);
        }
        Snapshot {
            ts,
            num_startups: n,
            tips_history: th,
        }
    }

    #[test]
    fn humanize_brackets() {
        assert_eq!(humanize_relative(5_000), "just now");
        assert_eq!(humanize_relative(60_000), "1 minute ago");
        assert_eq!(humanize_relative(3_600_000), "1 hour ago");
        assert_eq!(humanize_relative(86_400_000), "yesterday");
        assert_eq!(humanize_relative(3 * 86_400_000), "3 days ago");
        assert_eq!(humanize_relative(60 * 86_400_000), "2 months ago");
    }

    #[test]
    fn resolve_finds_matching_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let log = SnapshotLog::at(dir.path().join("snaps.jsonl"));
        log.record_if_due(1_000, snap(1_000, 100, &[("foo", 95)]))
            .unwrap();
        log.record_if_due(
            1_000 + SNAPSHOT_DEBOUNCE_MS + 1,
            snap(1_000 + SNAPSHOT_DEBOUNCE_MS + 1, 110, &[("foo", 105)]),
        )
        .unwrap();
        let now = 1_000 + 5 * SNAPSHOT_DEBOUNCE_MS;
        let r = log.resolve_last_seen("foo", 95, now).unwrap().unwrap();
        assert!(!r.exact_unknown);
        assert!(r.relative.contains("hour"));
    }

    #[test]
    fn resolve_returns_unknown_for_uncovered_count() {
        let dir = tempfile::tempdir().unwrap();
        let log = SnapshotLog::at(dir.path().join("snaps.jsonl"));
        log.record_if_due(1_000, snap(1_000, 100, &[("foo", 50)]))
            .unwrap();
        let r = log
            .resolve_last_seen("foo", 80, 5_000_000)
            .unwrap()
            .unwrap();
        assert!(r.exact_unknown);
    }

    #[test]
    fn debounce_skips_under_one_hour() {
        let dir = tempfile::tempdir().unwrap();
        let log = SnapshotLog::at(dir.path().join("snaps.jsonl"));
        let written = log.record_if_due(1_000, snap(1_000, 100, &[])).unwrap();
        assert!(written);
        let written2 = log
            .record_if_due(1_000 + 60_000, snap(1_000 + 60_000, 101, &[]))
            .unwrap();
        assert!(!written2);
    }

    #[test]
    fn cap_enforces_fifo() {
        let dir = tempfile::tempdir().unwrap();
        let log = SnapshotLog::at(dir.path().join("snaps.jsonl"));
        for i in 0..(SNAPSHOT_CAP as i64 + 5) {
            let ts = i * (SNAPSHOT_DEBOUNCE_MS + 1);
            log.record_if_due(ts, snap(ts, i as u32, &[])).unwrap();
        }
        let entries = log.read_all().unwrap();
        assert_eq!(entries.len(), SNAPSHOT_CAP);
    }
}
