//! Everything **other than transcripts** that `cleanupPeriodDays` ages
//! out of `~/.claude`.
//!
//! # Why this module exists
//!
//! `cleanupPeriodDays` is not a transcript setting. It is a global TTL,
//! and [`crate::cc_retention::TranscriptRisk`] deliberately counts only
//! `projects/` — the irreplaceable part, and the only part the pane can
//! describe in one sentence. That left the rest of the timer invisible:
//! the pane could say "no saved conversations are scheduled for
//! deletion" while `file-history/` and `uploads/` were being aged out on
//! the same cutoff.
//!
//! Scoping the sentence fixed the *over-claim*. This module fixes the
//! *gap*.
//!
//! # The two sweep shapes, which decide what to count
//!
//! Verified by reading the cleanup module of the installed **2.1.233**
//! binary, not the changelog — CC 2.1.117's release note announced three
//! of these directories and the binary shows a far larger set:
//!
//! - **`Files { ext }`** — the sweep unlinks *files* directly in the
//!   directory whose mtime is past the cutoff, filtered by extension
//!   (`""` means any). Count files.
//! - **`Subdirs`** — the sweep removes *immediate subdirectories* whose
//!   mtime is past the cutoff, recursively. Count subdirectories.
//!
//! Counting the wrong unit would report zero for half of these, which is
//! the failure this module exists to remove.
//!
//! # Content vs cache, and why the split is explicit
//!
//! Only [`SweptKind::Content`] rows are surfaced. A user who loses
//! `telemetry/` has lost nothing; a user who loses `uploads/` has lost
//! files they put there. Classifying is a judgement, so it is written
//! down per row and reviewable, rather than implied by an omission.
//!
//! `projects/` is absent on purpose — `cc_retention` owns it.

use std::path::{Path, PathBuf};

/// What the sweep deletes, and therefore what a scan must count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SweepUnit {
    /// Files directly in the directory, filtered by extension. `""`
    /// means every file regardless of extension.
    Files { ext: &'static str },
    /// Immediate subdirectories, removed recursively.
    Subdirs,
}

/// Whether losing this directory costs the user anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SweptKind {
    /// User work product. Worth reporting.
    Content,
    /// Cache or diagnostics — regenerated, or of no value once stale.
    /// Recorded so the exclusion is a decision rather than an oversight.
    Cache,
}

/// One directory CC ages out on the `cleanupPeriodDays` timer.
#[derive(Clone, Copy, Debug)]
pub struct SweptSpec {
    /// Path relative to the CC config dir. `/`-joined here and split on
    /// the way to a `PathBuf`, so this table stays readable and no
    /// separator is hardcoded into the lookup — see `rules/paths.md`.
    pub rel: &'static str,
    pub unit: SweepUnit,
    pub kind: SweptKind,
    /// What the user actually loses. Rendered in the UI, so it is
    /// written for a reader, not for a maintainer.
    pub what: &'static str,
}

/// The verified sweep table for CC 2.1.233.
///
/// Re-derive with `cargo xtask cc-drift` when the watchlist's
/// "`cleanupPeriodDays` sweep scope" row reports movement. Adding a row
/// here is how that row gets closed; deleting one needs the same
/// evidence.
pub const SWEPT: &[SweptSpec] = &[
    // ── content ────────────────────────────────────────────────────
    SweptSpec {
        rel: "tasks",
        unit: SweepUnit::Subdirs,
        kind: SweptKind::Content,
        what: "background task state",
    },
    SweptSpec {
        rel: "file-history",
        unit: SweepUnit::Subdirs,
        kind: SweptKind::Content,
        what: "file edit history",
    },
    SweptSpec {
        rel: "uploads",
        unit: SweepUnit::Subdirs,
        kind: SweptKind::Content,
        what: "files you uploaded",
    },
    SweptSpec {
        rel: "shares",
        unit: SweepUnit::Subdirs,
        kind: SweptKind::Content,
        what: "shared conversations",
    },
    SweptSpec {
        rel: "dump-prompts",
        unit: SweepUnit::Files { ext: "jsonl" },
        kind: SweptKind::Content,
        what: "exported prompts",
    },
    SweptSpec {
        rel: "shell-snapshots",
        unit: SweepUnit::Files { ext: "sh" },
        kind: SweptKind::Content,
        what: "shell environment snapshots",
    },
    SweptSpec {
        rel: "backups",
        unit: SweepUnit::Files { ext: "" },
        kind: SweptKind::Content,
        what: "settings backups",
    },
    SweptSpec {
        rel: "plans",
        unit: SweepUnit::Files { ext: "md" },
        kind: SweptKind::Content,
        what: "saved plans",
    },
    // ── cache / diagnostics: counted by nobody, listed by name ──────
    SweptSpec {
        rel: "telemetry",
        unit: SweepUnit::Files { ext: "json" },
        kind: SweptKind::Cache,
        what: "telemetry",
    },
    SweptSpec {
        rel: "traces",
        unit: SweepUnit::Files { ext: "json" },
        kind: SweptKind::Cache,
        what: "traces",
    },
    SweptSpec {
        rel: "startup-perf",
        unit: SweepUnit::Files { ext: "" },
        kind: SweptKind::Cache,
        what: "startup timings",
    },
    SweptSpec {
        rel: "mcp-discovery-cache",
        unit: SweepUnit::Files { ext: "json" },
        kind: SweptKind::Cache,
        what: "MCP discovery cache",
    },
    SweptSpec {
        rel: "feedback-bundles",
        unit: SweepUnit::Files { ext: "zip" },
        kind: SweptKind::Cache,
        what: "feedback bundles",
    },
    SweptSpec {
        rel: "session-env",
        unit: SweepUnit::Subdirs,
        kind: SweptKind::Cache,
        what: "per-session environment",
    },
    SweptSpec {
        rel: "jobs",
        unit: SweepUnit::Subdirs,
        kind: SweptKind::Cache,
        what: "finished job records",
    },
];

/// Counts for one swept directory.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SweptDir {
    pub rel: String,
    pub what: String,
    pub kind: SweptKind,
    /// Entries present, in the unit CC deletes.
    pub entries: u64,
    /// Entries already past the cutoff — gone at CC's next launch.
    pub already_deletable: u64,
}

/// What `cleanupPeriodDays` is aging out beyond the transcript tree.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SweptElsewhere {
    /// Content directories that exist on this machine and hold at least
    /// one entry. Empty directories are omitted — render-if-nonzero.
    pub dirs: Vec<SweptDir>,
    /// Some directory could not be read. The counts are a floor; never
    /// render them as a total while this is set.
    pub scan_incomplete: bool,
    /// Cache/diagnostic directories skipped by name, so the UI can say
    /// what it chose not to count instead of leaving it implied.
    pub cache_dirs_skipped: u64,
}

impl SweptElsewhere {
    pub fn total_entries(&self) -> u64 {
        self.dirs.iter().map(|d| d.entries).sum()
    }
    pub fn total_deletable(&self) -> u64 {
        self.dirs.iter().map(|d| d.already_deletable).sum()
    }
}

/// Scan the non-transcript sweep targets under `config_dir`.
///
/// `cutoff_ms` is the same cutoff `cc_retention` computes, passed in
/// rather than recomputed so the two surfaces can never disagree about
/// when the guillotine falls. Pass `None` when cleanup is suppressed:
/// nothing is at risk then, and the counts still report what exists.
pub fn scan_swept_in(config_dir: &Path, cutoff_ms: Option<i64>) -> SweptElsewhere {
    let mut out = SweptElsewhere::default();
    for spec in SWEPT {
        if spec.kind == SweptKind::Cache {
            out.cache_dirs_skipped += 1;
            continue;
        }
        // Split on '/' and re-join, so the table never embeds a
        // platform separator into a single component.
        let mut dir: PathBuf = config_dir.to_path_buf();
        for part in spec.rel.split('/') {
            dir.push(part);
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            // A directory that does not exist is a real zero, not a
            // failed read: CC creates these lazily.
            if dir.exists() {
                out.scan_incomplete = true;
            }
            continue;
        };
        let mut entries = 0u64;
        let mut deletable = 0u64;
        for ent in rd {
            let Ok(ent) = ent else {
                out.scan_incomplete = true;
                continue;
            };
            let Ok(ft) = ent.file_type() else {
                out.scan_incomplete = true;
                continue;
            };
            let matches = match spec.unit {
                SweepUnit::Subdirs => ft.is_dir(),
                SweepUnit::Files { ext } => {
                    ft.is_file()
                        && (ext.is_empty()
                            || ent
                                .path()
                                .extension()
                                .is_some_and(|e| e.eq_ignore_ascii_case(ext)))
                }
            };
            if !matches {
                continue;
            }
            entries += 1;
            let Ok(md) = ent.metadata() else {
                out.scan_incomplete = true;
                continue;
            };
            let Some(mtime) = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .and_then(|d| i64::try_from(d.as_millis()).ok())
            else {
                out.scan_incomplete = true;
                continue;
            };
            if cutoff_ms.is_some_and(|c| mtime < c) {
                deletable += 1;
            }
        }
        if entries > 0 {
            out.dirs.push(SweptDir {
                rel: spec.rel.to_string(),
                what: spec.what.to_string(),
                kind: spec.kind,
                entries,
                already_deletable: deletable,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn touch(p: &Path, age_ms_ago: i64, now: i64) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"x").unwrap();
        let t = std::time::UNIX_EPOCH
            + std::time::Duration::from_millis((now - age_ms_ago).max(0) as u64);
        filetime::set_file_mtime(p, filetime::FileTime::from_system_time(t)).unwrap();
    }

    fn mkdir(p: &Path, age_ms_ago: i64, now: i64) {
        fs::create_dir_all(p).unwrap();
        let t = std::time::UNIX_EPOCH
            + std::time::Duration::from_millis((now - age_ms_ago).max(0) as u64);
        filetime::set_file_mtime(p, filetime::FileTime::from_system_time(t)).unwrap();
    }

    const DAY: i64 = 24 * 60 * 60 * 1000;

    /// The whole point: counting files where CC deletes subdirectories
    /// (or vice versa) reports zero and reads as "nothing here".
    #[test]
    fn each_directory_is_counted_in_the_unit_cc_deletes() {
        let now = 1_800_000_000_000i64;
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();

        // Subdirs unit: a loose file must NOT count, a subdir must.
        mkdir(&cfg.join("file-history").join("a"), 60 * DAY, now);
        touch(&cfg.join("file-history").join("loose.txt"), 60 * DAY, now);

        // Files unit with an extension filter.
        touch(&cfg.join("shell-snapshots").join("s1.sh"), 60 * DAY, now);
        touch(&cfg.join("shell-snapshots").join("notes.md"), 60 * DAY, now);

        let got = scan_swept_in(cfg, Some(now - 30 * DAY));
        let fh = got.dirs.iter().find(|d| d.rel == "file-history").unwrap();
        assert_eq!(fh.entries, 1, "subdirs only; the loose file is not a unit");
        let sh = got
            .dirs
            .iter()
            .find(|d| d.rel == "shell-snapshots")
            .unwrap();
        assert_eq!(sh.entries, 1, "only .sh matches the sweep's filter");
    }

    #[test]
    fn entries_past_the_cutoff_are_counted_as_deletable() {
        let now = 1_800_000_000_000i64;
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        mkdir(&cfg.join("uploads").join("old"), 60 * DAY, now);
        mkdir(&cfg.join("uploads").join("fresh"), DAY, now);
        let got = scan_swept_in(cfg, Some(now - 30 * DAY));
        let up = got.dirs.iter().find(|d| d.rel == "uploads").unwrap();
        assert_eq!(up.entries, 2);
        assert_eq!(up.already_deletable, 1);
    }

    /// Cleanup suppressed ⇒ no cutoff ⇒ nothing is at risk, but what
    /// exists is still reported. Mirrors `TranscriptRisk`'s behaviour so
    /// the two halves of the pane cannot contradict each other.
    #[test]
    fn a_suppressed_cleanup_puts_nothing_at_risk_but_still_counts() {
        let now = 1_800_000_000_000i64;
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        mkdir(&cfg.join("uploads").join("old"), 900 * DAY, now);
        let got = scan_swept_in(cfg, None);
        let up = got.dirs.iter().find(|d| d.rel == "uploads").unwrap();
        assert_eq!(up.entries, 1);
        assert_eq!(up.already_deletable, 0);
    }

    #[test]
    fn cache_directories_are_skipped_and_counted_as_skipped() {
        let now = 1_800_000_000_000i64;
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        touch(&cfg.join("telemetry").join("t.json"), 60 * DAY, now);
        let got = scan_swept_in(cfg, Some(now - 30 * DAY));
        assert!(got.dirs.iter().all(|d| d.rel != "telemetry"));
        assert_eq!(
            got.cache_dirs_skipped,
            SWEPT.iter().filter(|s| s.kind == SweptKind::Cache).count() as u64,
            "the UI states how many it chose not to count"
        );
    }

    /// An absent directory is a real zero — CC creates these lazily, so
    /// treating "missing" as "unreadable" would flag every fresh install
    /// as an incomplete scan.
    #[test]
    fn a_missing_directory_is_not_an_incomplete_scan() {
        let tmp = TempDir::new().unwrap();
        let got = scan_swept_in(tmp.path(), Some(0));
        assert!(got.dirs.is_empty());
        assert!(!got.scan_incomplete);
    }

    /// Empty directories are omitted so the pane never renders
    /// "0 entries" rows — design.md's render-if-nonzero rule.
    #[test]
    fn empty_directories_are_omitted() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("uploads")).unwrap();
        let got = scan_swept_in(tmp.path(), Some(0));
        assert!(got.dirs.is_empty());
    }

    /// Every content row must carry prose a user can read; a row whose
    /// `what` is a directory name has not been written for anyone.
    #[test]
    fn every_content_row_explains_what_is_lost() {
        for s in SWEPT.iter().filter(|s| s.kind == SweptKind::Content) {
            assert!(!s.what.is_empty(), "{} has no description", s.rel);
            assert!(
                s.what != s.rel,
                "{} describes itself with its own path",
                s.rel
            );
        }
    }
}
