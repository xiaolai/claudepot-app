//! Delta-export state — per-`(peer, project)` file fingerprints.
//!
//! Lets a repeated export ship only what changed since the last export
//! to that peer, instead of a full bundle every time.
//!
//! ## Why a high-water mark would be wrong
//!
//! The obvious design — record a timestamp, export everything newer —
//! fails on this data. `claudepot session slim` rewrites a transcript
//! **in place**, and CC's `cleanupPeriodDays` deletes whole files on a
//! sliding window. Neither is "after" a watermark in any useful sense:
//! a slimmed file is *smaller* than the copy the peer holds, and a
//! deleted file has no timestamp at all. A watermark skips both, so
//! the peer keeps a stale fat copy forever and never learns about the
//! deletion.
//!
//! Fingerprints catch both. `(size, mtime_ns)` diverging in *either*
//! direction marks the file for re-export, and a path that was
//! fingerprinted but is now absent becomes a tombstone.
//!
//! ## Why tombstones do not delete
//!
//! A tombstone says "this file left the source", never "remove it on
//! the target". The source may have run a retention sweep the target's
//! user does not want mirrored — deleting their only remaining copy of
//! a transcript because another machine aged it out is exactly the
//! silent data loss this whole plan exists to avoid. The importer
//! surfaces them; a human decides.
//!
//! ## Storage
//!
//! `~/.claudepot/migrate-peers.json`, via `json_store` (so a corrupt
//! file moves aside and starts empty rather than being fatal at boot).
//! This is **transport state, not cache**: it must survive a
//! `sessions.db` rebuild, because rebuilding a cache would otherwise
//! silently re-send every file to every peer. That is why it is its
//! own file and not a table in `sessions.db`, whose documented remedy
//! is "delete and rebuild".

use crate::json_store;
use crate::session_index::diff::{self, DiffPlan, IndexTuple};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;

/// Bundle entry listing paths that left the source since the last
/// export to this peer.
pub const TOMBSTONES_ENTRY: &str = "tombstones.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileFingerprint {
    /// Slug-relative, `/`-joined.
    pub path: String,
    pub size: u64,
    pub mtime_ns: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PeerRecord {
    /// Keyed by source cwd.
    #[serde(default)]
    pub projects: BTreeMap<String, Vec<FileFingerprint>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerStore {
    pub schema_version: u32,
    #[serde(default)]
    pub peers: BTreeMap<String, PeerRecord>,
}

impl Default for PeerStore {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            peers: BTreeMap::new(),
        }
    }
}

/// A schema this build cannot read. Treated as corruption by
/// `json_store`, which moves the file aside and starts empty — the
/// safe direction: worst case is one full re-export to each peer.
#[derive(Debug)]
pub struct PeerStoreInvalid(String);

impl std::fmt::Display for PeerStoreInvalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl json_store::Validate for PeerStore {
    type Error = PeerStoreInvalid;

    fn validate(&self) -> Result<(), Self::Error> {
        if self.schema_version == 0 || self.schema_version > SCHEMA_VERSION {
            return Err(PeerStoreInvalid(format!(
                "unsupported schema_version {} (this build reads <= {SCHEMA_VERSION})",
                self.schema_version
            )));
        }
        Ok(())
    }
}

pub fn state_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join("migrate-peers.json")
}

pub fn load() -> PeerStore {
    json_store::load_or_default(&state_path(), "migrate_peers")
}

pub fn save(store: &PeerStore) -> std::io::Result<()> {
    json_store::save(&state_path(), store).map_err(|e| std::io::Error::other(format!("{e:?}")))
}

/// Fingerprint every file under `slug_dir`, slug-relative.
///
/// `inode` is left at 0: it is meaningless across the export boundary
/// and `diff_fs_vs_db` only compares `(size, mtime_ns)`.
pub fn fingerprint_dir(slug_dir: &Path) -> Vec<IndexTuple> {
    let mut out = Vec::new();
    for rel in crate::migrate::apply::collect_dir_inventory(slug_dir) {
        let mut abs = slug_dir.to_path_buf();
        for seg in rel.split('/').filter(|s| !s.is_empty()) {
            abs.push(seg);
        }
        let Ok(meta) = std::fs::metadata(&abs) else {
            continue;
        };
        out.push(IndexTuple {
            file_path: rel,
            size: meta.len(),
            mtime_ns: mtime_ns(&meta),
            inode: 0,
        });
    }
    out.sort_by(|a, b| a.file_path.cmp(&b.file_path));
    out
}

fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// What to ship for `(peer, project)` given the current on-disk state.
///
/// `to_upsert` is new-or-changed (in either direction, so a slimmed
/// file is included); `to_delete` is the tombstone list.
pub fn delta(
    store: &PeerStore,
    peer_id: &str,
    source_cwd: &str,
    fs_now: &[IndexTuple],
) -> DiffPlan {
    let recorded: Vec<IndexTuple> = store
        .peers
        .get(peer_id)
        .and_then(|r| r.projects.get(source_cwd))
        .map(|fps| {
            fps.iter()
                .map(|f| IndexTuple {
                    file_path: f.path.clone(),
                    size: f.size,
                    mtime_ns: f.mtime_ns,
                    inode: 0,
                })
                .collect()
        })
        .unwrap_or_default();
    diff::diff_fs_vs_db(fs_now, &recorded)
}

/// Replace the recorded fingerprints for `(peer, project)`. Call only
/// after the bundle has been written — recording first would claim
/// files were sent that a failed export never shipped.
pub fn record(store: &mut PeerStore, peer_id: &str, source_cwd: &str, fs_now: &[IndexTuple]) {
    let rec = store.peers.entry(peer_id.to_string()).or_default();
    rec.projects.insert(
        source_cwd.to_string(),
        fs_now
            .iter()
            .map(|t| FileFingerprint {
                path: t.file_path.clone(),
                size: t.size,
                mtime_ns: t.mtime_ns,
            })
            .collect(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuple(path: &str, size: u64, mtime_ns: i64) -> IndexTuple {
        IndexTuple {
            file_path: path.to_string(),
            size,
            mtime_ns,
            inode: 0,
        }
    }

    #[test]
    fn first_export_to_a_peer_ships_everything() {
        let store = PeerStore::default();
        let now = vec![tuple("a.jsonl", 10, 1), tuple("b.jsonl", 20, 2)];
        let plan = delta(&store, "laptop", "/proj", &now);
        assert_eq!(plan.to_upsert, vec!["a.jsonl", "b.jsonl"]);
        assert!(plan.to_delete.is_empty());
    }

    #[test]
    fn unchanged_files_are_not_reshipped() {
        let mut store = PeerStore::default();
        let now = vec![tuple("a.jsonl", 10, 1)];
        record(&mut store, "laptop", "/proj", &now);
        let plan = delta(&store, "laptop", "/proj", &now);
        assert!(plan.to_upsert.is_empty());
        assert!(plan.to_delete.is_empty());
    }

    /// The property a high-water mark gets wrong: `slim` makes a file
    /// smaller, and it must still re-export.
    #[test]
    fn a_shrunk_file_is_reshipped() {
        let mut store = PeerStore::default();
        record(&mut store, "laptop", "/proj", &[tuple("a.jsonl", 900, 5)]);
        // Same mtime, smaller — the shape `slim` leaves behind if the
        // clock is coarse.
        let plan = delta(&store, "laptop", "/proj", &[tuple("a.jsonl", 100, 5)]);
        assert_eq!(plan.to_upsert, vec!["a.jsonl"]);
    }

    #[test]
    fn a_removed_file_becomes_a_tombstone() {
        let mut store = PeerStore::default();
        record(
            &mut store,
            "laptop",
            "/proj",
            &[tuple("a.jsonl", 10, 1), tuple("gone.jsonl", 10, 1)],
        );
        let plan = delta(&store, "laptop", "/proj", &[tuple("a.jsonl", 10, 1)]);
        assert!(plan.to_upsert.is_empty());
        assert_eq!(plan.to_delete, vec!["gone.jsonl"]);
    }

    #[test]
    fn peers_and_projects_are_tracked_independently() {
        let mut store = PeerStore::default();
        let now = vec![tuple("a.jsonl", 10, 1)];
        record(&mut store, "laptop", "/proj", &now);
        // A different peer has seen nothing.
        assert_eq!(delta(&store, "desktop", "/proj", &now).to_upsert.len(), 1);
        // A different project on the same peer likewise.
        assert_eq!(delta(&store, "laptop", "/other", &now).to_upsert.len(), 1);
    }

    #[test]
    fn fingerprint_dir_walks_nested_files() {
        let td = tempfile::tempdir().unwrap();
        let root = td.path();
        std::fs::write(root.join("a.jsonl"), "hello").unwrap();
        std::fs::create_dir_all(root.join("sid/subagents")).unwrap();
        std::fs::write(root.join("sid/subagents/b.jsonl"), "sub").unwrap();

        let fps = fingerprint_dir(root);
        let paths: Vec<&str> = fps.iter().map(|t| t.file_path.as_str()).collect();
        assert_eq!(paths, vec!["a.jsonl", "sid/subagents/b.jsonl"]);
        assert_eq!(fps[0].size, 5);
    }

    #[test]
    fn store_round_trips_through_serde() {
        let mut store = PeerStore::default();
        record(&mut store, "laptop", "/proj", &[tuple("a.jsonl", 10, 1)]);
        let text = serde_json::to_string(&store).unwrap();
        let back: PeerStore = serde_json::from_str(&text).unwrap();
        assert_eq!(back.schema_version, SCHEMA_VERSION);
        assert_eq!(back.peers["laptop"].projects["/proj"][0].size, 10);
    }
}
