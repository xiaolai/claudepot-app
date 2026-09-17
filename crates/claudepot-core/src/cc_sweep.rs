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
//! # The sweep shapes, which decide what to count
//!
//! Re-read from the cleanup module of the installed **2.1.274** binary
//! (its `tengu_retention_sweep` orchestrator runs 42 sweep functions),
//! not the changelog — CC 2.1.117's release note announced three of
//! these directories and the binary shows a far larger set:
//!
//! - **`Files { ext }`** — the sweep unlinks *files* directly in the
//!   directory whose mtime is past the cutoff, filtered by extension
//!   (`""` means any). Count files.
//! - **`Subdirs`** — the sweep removes *immediate subdirectories* whose
//!   mtime is past the cutoff, recursively. Count subdirectories.
//! - **`Entries { file_ext }`** — both of the above in one directory:
//!   every immediate subdirectory, plus the files matching `file_ext`.
//! - **`Tree`** — every file anywhere below, each by its own mtime.
//!   Count files.
//!
//! Counting the wrong unit would report zero for half of these, which is
//! the failure this module exists to remove.
//!
//! A `*` path component means "each immediate subdirectory here", for
//! the sweeps that walk `projects/<slug>/…` and `teams/<team>/…`.
//!
//! # A cap below `cleanupPeriodDays`
//!
//! A few sweeps pass their own maximum age, and CC takes the *smaller*
//! of it and `cleanupPeriodDays` (`n_(maxAgeDays)` in the binary).
//! `dump-prompts` goes after 3 days whatever the setting says, so a
//! count against the setting's cutoff alone would under-report it.
//! [`SweptSpec::max_age_days`] carries that cap.
//!
//! # What this table does not cover
//!
//! Only directories under the config dir, because that is what
//! [`scan_swept_in`] walks. The same sweep also reaches:
//!
//! - the system temp dir — `cc-transcript-*`, `auto-mode-builtins-*`,
//!   `cache-break-*`, `speculation/`, `bash-edit-diff/` (2-day cap), and
//!   `cc-daemon-*` folders (CC 2.1.257);
//! - `~/.claude/bridge-spawn` and `~/.claude/state/served-calls`, joined
//!   onto the *home* directory rather than the config dir;
//! - the plugin cache's `store/`, which `CLAUDE_CODE_PLUGIN_CACHE_DIR`
//!   can move anywhere;
//! - single files — `hfi-auth.json`, `mcp-needs-auth-cache.json`,
//!   `cache/team-discovery.json`, `state/device-unbound-creates.json`,
//!   `daemon.log`, `daemon/roster.json`, the settings-review store.
//!
//! All of it is cache, scratch or diagnostics.
//!
//! # Content vs cache, and why the split is explicit
//!
//! Only [`SweptKind::Content`] rows are surfaced. A user who loses
//! `telemetry/` has lost nothing; a user who loses `uploads/` has lost
//! files they put there. Classifying is a judgement, so it is written
//! down per row and reviewable, rather than implied by an omission.
//! Synced skills and plugins are cache on that test: CC moves a copy it
//! has not refreshed within the window to `.trash` (CC 2.1.271), and the
//! copy's source is the claude.ai account, which still has it.
//!
//! `projects/<slug>/*.jsonl` and the session folders beside them are
//! absent on purpose — `cc_retention` owns them. `tiny_memory` is a
//! sibling of those folders rather than part of a session, so it is
//! listed here.

use std::path::{Path, PathBuf};

/// What the sweep deletes, and therefore what a scan must count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SweepUnit {
    /// Files directly in the directory, filtered by extension. `""`
    /// means every file regardless of extension.
    Files { ext: &'static str },
    /// Immediate subdirectories, removed recursively.
    Subdirs,
    /// Immediate subdirectories, plus files matching `file_ext` (`""`
    /// means every file).
    Entries { file_ext: &'static str },
    /// Every file at any depth, each by its own mtime.
    Tree,
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
    /// Stable identifier, used by the GUI as its catalog key so the
    /// description can be localized. Never rendered.
    pub id: &'static str,
    /// Path relative to the CC config dir. `/`-joined here and split on
    /// the way to a `PathBuf`, so this table stays readable and no
    /// separator is hardcoded into the lookup — see `rules/paths.md`.
    /// A `*` component stands for each immediate subdirectory.
    pub rel: &'static str,
    pub unit: SweepUnit,
    pub kind: SweptKind,
    /// What the user actually loses, in English. The GUI shows its own
    /// catalog entry for [`Self::id`] and falls back to this.
    pub what: &'static str,
    /// CC's own cap for this sweep, in days. The effective window is
    /// the smaller of this and `cleanupPeriodDays`.
    pub max_age_days: Option<i64>,
}

const fn row(
    id: &'static str,
    rel: &'static str,
    unit: SweepUnit,
    kind: SweptKind,
    what: &'static str,
) -> SweptSpec {
    SweptSpec {
        id,
        rel,
        unit,
        kind,
        what,
        max_age_days: None,
    }
}

const fn capped(spec: SweptSpec, days: i64) -> SweptSpec {
    SweptSpec {
        max_age_days: Some(days),
        ..spec
    }
}

use SweepUnit::{Entries, Files, Subdirs, Tree};
use SweptKind::{Cache, Content};

/// The verified sweep table for CC 2.1.274.
///
/// Re-derive with `cargo xtask cc-drift` when the watchlist's
/// "`cleanupPeriodDays` sweep scope" row reports movement. Adding a row
/// here is how that row gets closed; deleting one needs the same
/// evidence.
pub const SWEPT: &[SweptSpec] = &[
    // ── content ────────────────────────────────────────────────────
    row("tasks", "tasks", Subdirs, Content, "background task state"),
    row(
        "fileHistory",
        "file-history",
        Subdirs,
        Content,
        "file edit history",
    ),
    row("uploads", "uploads", Subdirs, Content, "files you uploaded"),
    row(
        "shares",
        "shares",
        Entries { file_ext: "zip" },
        Content,
        "shared conversations",
    ),
    capped(
        row(
            "dumpPrompts",
            "dump-prompts",
            Files { ext: "jsonl" },
            Content,
            "exported prompts",
        ),
        3,
    ),
    row(
        "shellSnapshots",
        "shell-snapshots",
        Files { ext: "sh" },
        Content,
        "shell environment snapshots",
    ),
    row(
        "backups",
        "backups",
        Files { ext: "" },
        Content,
        "settings backups",
    ),
    row(
        "plans",
        "plans",
        Files { ext: "md" },
        Content,
        "saved plans",
    ),
    row(
        "todos",
        "todos",
        Entries { file_ext: "" },
        Content,
        "to-do lists",
    ),
    row(
        "usageReports",
        "usage-data",
        Files { ext: "html" },
        Content,
        "usage insight reports",
    ),
    capped(
        row(
            "feedbackDrafts",
            "feedback/drafts",
            Files { ext: "json" },
            Content,
            "unsent feedback drafts",
        ),
        30,
    ),
    row(
        "tinyMemory",
        "projects/*/tiny_memory",
        Tree,
        Content,
        "per-project tiny_memory files",
    ),
    // ── cache / diagnostics: counted by nobody, listed by name ──────
    row(
        "telemetry",
        "telemetry",
        Files { ext: "json" },
        Cache,
        "telemetry",
    ),
    row("traces", "traces", Files { ext: "json" }, Cache, "traces"),
    row(
        "startupPerf",
        "startup-perf",
        Files { ext: "" },
        Cache,
        "startup timings",
    ),
    row("debug", "debug", Files { ext: "" }, Cache, "debug logs"),
    row("logs", "logs", Entries { file_ext: "" }, Cache, "logs"),
    row(
        "statsig",
        "statsig",
        Entries { file_ext: "" },
        Cache,
        "feature-flag cache",
    ),
    row(
        "mcpDiscoveryCache",
        "mcp-discovery-cache",
        Files { ext: "json" },
        Cache,
        "MCP discovery cache",
    ),
    row(
        "mcpSkillArchives",
        "mcp-skill-archives",
        Subdirs,
        Cache,
        "MCP skill archives",
    ),
    row(
        "modelCatalog",
        "cache/model-catalog",
        Files { ext: "json" },
        Cache,
        "model catalog cache",
    ),
    row(
        "feedbackBundles",
        "feedback-bundles",
        Files { ext: "zip" },
        Cache,
        "feedback bundles",
    ),
    row(
        "sessionEnv",
        "session-env",
        Subdirs,
        Cache,
        "per-session environment",
    ),
    row("jobs", "jobs", Subdirs, Cache, "finished job records"),
    row(
        "jobsSettled",
        "jobs/settled",
        Files { ext: "json" },
        Cache,
        "settled job records",
    ),
    row(
        "daemonDispatch",
        "daemon/dispatch",
        Files { ext: "json" },
        Cache,
        "daemon dispatch records",
    ),
    row(
        "daemonRejected",
        "daemon/dispatch/rejected",
        Files { ext: "json" },
        Cache,
        "rejected daemon dispatches",
    ),
    row(
        "daemonAuth",
        "daemon/auth",
        Files { ext: "json" },
        Cache,
        "daemon auth records",
    ),
    row(
        "daemonHostManaged",
        "daemon/host-managed",
        Files { ext: "" },
        Cache,
        "host-managed job markers",
    ),
    capped(
        row(
            "fileTransfers",
            "file-transfers",
            Files { ext: "" },
            Cache,
            "file transfers",
        ),
        1,
    ),
    row(
        "usageFacets",
        "usage-data/facets",
        Files { ext: "json" },
        Cache,
        "usage analysis cache",
    ),
    row(
        "usageSessionMeta",
        "usage-data/session-meta",
        Files { ext: "json" },
        Cache,
        "usage session cache",
    ),
    row(
        "teamInboxes",
        "teams/*/inboxes",
        Files { ext: "json" },
        Cache,
        "agent-team mailboxes",
    ),
    row(
        "forkSeeds",
        "remote-control/fork-seeds",
        Files { ext: "json" },
        Cache,
        "remote-control fork seeds",
    ),
    row(
        "memoryProposals",
        "projects/*/memory/proposals",
        Files { ext: "md" },
        Cache,
        "unreviewed skill proposals",
    ),
    row(
        "syncedSkills",
        "skills/synced",
        Subdirs,
        Cache,
        "synced claude.ai skills",
    ),
    row(
        "skillsTrash",
        "skills/.trash",
        Subdirs,
        Cache,
        "retired synced skills",
    ),
    row(
        "skillsStaging",
        "skills/.staging",
        Subdirs,
        Cache,
        "skill staging",
    ),
    row(
        "syncedSkillsStaging",
        "skills/synced/.staging",
        Subdirs,
        Cache,
        "synced skill staging",
    ),
    row(
        "syncedPlugins",
        "plugins/synced",
        Subdirs,
        Cache,
        "synced claude.ai plugins",
    ),
    row(
        "pluginsTrash",
        "plugins/.trash",
        Subdirs,
        Cache,
        "retired synced plugins",
    ),
    row(
        "syncedPluginsStaging",
        "plugins/synced/.staging",
        Subdirs,
        Cache,
        "synced plugin staging",
    ),
];

/// Counts for one swept directory.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SweptDir {
    /// [`SweptSpec::id`] — the GUI's catalog key.
    pub id: String,
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
/// `now_ms` is only used to apply a row's own [`SweptSpec::max_age_days`].
pub fn scan_swept_in(config_dir: &Path, now_ms: i64, cutoff_ms: Option<i64>) -> SweptElsewhere {
    let mut out = SweptElsewhere::default();
    for spec in SWEPT {
        if spec.kind == SweptKind::Cache {
            out.cache_dirs_skipped += 1;
            continue;
        }
        // CC takes the smaller window, i.e. the later cutoff.
        let cutoff = cutoff_ms.map(|c| match spec.max_age_days {
            Some(days) => c.max(now_ms.saturating_sub(days.saturating_mul(DAY_MS))),
            None => c,
        });
        let mut tally = Tally::default();
        for dir in expand(config_dir, spec.rel, &mut out.scan_incomplete) {
            count_dir(&dir, spec.unit, cutoff, &mut tally);
        }
        out.scan_incomplete |= tally.incomplete;
        if tally.entries > 0 {
            out.dirs.push(SweptDir {
                id: spec.id.to_string(),
                rel: spec.rel.to_string(),
                what: spec.what.to_string(),
                kind: spec.kind,
                entries: tally.entries,
                already_deletable: tally.deletable,
            });
        }
    }
    out
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Default)]
struct Tally {
    entries: u64,
    deletable: u64,
    incomplete: bool,
}

/// Resolve `rel` to the directories it names, expanding each `*`
/// component over the immediate subdirectories at that level.
///
/// A missing directory yields nothing and is not an incomplete scan —
/// CC creates these lazily. One that exists and cannot be listed is.
fn expand(config_dir: &Path, rel: &str, incomplete: &mut bool) -> Vec<PathBuf> {
    let mut dirs = vec![config_dir.to_path_buf()];
    for part in rel.split('/') {
        let mut next = Vec::new();
        for dir in dirs {
            if part != "*" {
                next.push(dir.join(part));
                continue;
            }
            let Ok(rd) = std::fs::read_dir(&dir) else {
                if dir.exists() {
                    *incomplete = true;
                }
                continue;
            };
            for ent in rd {
                match ent {
                    Ok(ent) if ent.file_type().is_ok_and(|t| t.is_dir()) => next.push(ent.path()),
                    Ok(_) => {}
                    Err(_) => *incomplete = true,
                }
            }
        }
        dirs = next;
    }
    dirs
}

/// Count one directory's entries in `unit`, and those past `cutoff`.
fn count_dir(dir: &Path, unit: SweepUnit, cutoff: Option<i64>, tally: &mut Tally) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        if dir.exists() {
            tally.incomplete = true;
        }
        return;
    };
    for ent in rd {
        let Ok(ent) = ent else {
            tally.incomplete = true;
            continue;
        };
        let Ok(ft) = ent.file_type() else {
            tally.incomplete = true;
            continue;
        };
        let path = ent.path();
        let ext_matches = |ext: &str| {
            ext.is_empty()
                || path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case(ext))
        };
        let matches = match unit {
            SweepUnit::Subdirs => ft.is_dir(),
            SweepUnit::Files { ext } => ft.is_file() && ext_matches(ext),
            SweepUnit::Entries { file_ext } => {
                ft.is_dir() || (ft.is_file() && ext_matches(file_ext))
            }
            SweepUnit::Tree => {
                if ft.is_dir() {
                    count_dir(&path, unit, cutoff, tally);
                    continue;
                }
                true
            }
        };
        if !matches {
            continue;
        }
        tally.entries += 1;
        // `symlink_metadata` for a tree entry: CC unlinks a non-file
        // there by its own lstat mtime rather than following it.
        let md = if unit == SweepUnit::Tree && !ft.is_file() {
            std::fs::symlink_metadata(&path)
        } else {
            ent.metadata()
        };
        let Some(mtime) = md
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| i64::try_from(d.as_millis()).ok())
        else {
            tally.incomplete = true;
            continue;
        };
        if cutoff.is_some_and(|c| mtime < c) {
            tally.deletable += 1;
        }
    }
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

        let got = scan_swept_in(cfg, now, Some(now - 30 * DAY));
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
        let got = scan_swept_in(cfg, now, Some(now - 30 * DAY));
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
        let got = scan_swept_in(cfg, now, None);
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
        let got = scan_swept_in(cfg, now, Some(now - 30 * DAY));
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
        let got = scan_swept_in(tmp.path(), 0, Some(0));
        assert!(got.dirs.is_empty());
        assert!(!got.scan_incomplete);
    }

    /// Empty directories are omitted so the pane never renders
    /// "0 entries" rows — design.md's render-if-nonzero rule.
    #[test]
    fn empty_directories_are_omitted() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("uploads")).unwrap();
        let got = scan_swept_in(tmp.path(), 0, Some(0));
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

    /// `id` is a catalog key and `rel` a React key: both must be
    /// unique, and `id` must survive i18next's `.` key separator.
    #[test]
    fn ids_and_paths_are_unique_and_ids_are_key_safe() {
        let mut ids = std::collections::BTreeSet::new();
        let mut rels = std::collections::BTreeSet::new();
        for s in SWEPT {
            assert!(ids.insert(s.id), "duplicate id {}", s.id);
            assert!(rels.insert(s.rel), "duplicate rel {}", s.rel);
            assert!(
                !s.id.is_empty() && s.id.chars().all(|c| c.is_ascii_alphanumeric()),
                "{} is not a plain catalog key",
                s.id
            );
        }
    }

    /// The pane shows the catalog's text for a content row, and would
    /// show this table's English in a Chinese UI for any row missing
    /// from a catalog — which `pnpm check:catalogs` cannot see, because
    /// it compares the catalogs with each other and not with this table.
    #[test]
    fn every_content_row_has_a_description_in_every_catalog() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/locales");
        for locale in ["en", "zh-CN"] {
            let path = root.join(locale).join("settings.json");
            let raw =
                fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
            let what = &json["retention"]["swept"]["what"];
            for s in SWEPT.iter().filter(|s| s.kind == SweptKind::Content) {
                let text = what[s.id].as_str().unwrap_or("");
                assert!(
                    !text.is_empty(),
                    "{locale} has no retention.swept.what.{}",
                    s.id
                );
            }
        }
    }

    #[test]
    fn a_row_with_its_own_cap_uses_the_shorter_window() {
        // dump-prompts goes after 3 days whatever cleanupPeriodDays
        // says. A 10-day-old export is gone at the next launch even
        // under a 30-day setting.
        let now = 1_800_000_000_000i64;
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        touch(&cfg.join("dump-prompts").join("old.jsonl"), 10 * DAY, now);
        touch(&cfg.join("dump-prompts").join("new.jsonl"), DAY, now);
        let got = scan_swept_in(cfg, now, Some(now - 30 * DAY));
        let dp = got.dirs.iter().find(|d| d.id == "dumpPrompts").unwrap();
        assert_eq!((dp.entries, dp.already_deletable), (2, 1));
        // A setting shorter than the cap wins instead.
        let got = scan_swept_in(cfg, now, Some(now - DAY / 2));
        let dp = got.dirs.iter().find(|d| d.id == "dumpPrompts").unwrap();
        assert_eq!(dp.already_deletable, 2);
    }

    #[test]
    fn an_entries_row_counts_subdirectories_and_matching_files() {
        let now = 1_800_000_000_000i64;
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        mkdir(&cfg.join("shares").join("s1"), 60 * DAY, now);
        touch(&cfg.join("shares").join("s2.zip"), 60 * DAY, now);
        touch(&cfg.join("shares").join("readme.txt"), 60 * DAY, now);
        let got = scan_swept_in(cfg, now, Some(now - 30 * DAY));
        let sh = got.dirs.iter().find(|d| d.id == "shares").unwrap();
        assert_eq!((sh.entries, sh.already_deletable), (2, 2));
    }

    #[test]
    fn a_wildcard_tree_row_counts_files_at_any_depth_in_every_project() {
        let now = 1_800_000_000_000i64;
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        let projects = cfg.join("projects");
        touch(
            &projects
                .join("a")
                .join("tiny_memory")
                .join("n")
                .join("x.md"),
            60 * DAY,
            now,
        );
        touch(
            &projects.join("b").join("tiny_memory").join("y.md"),
            DAY,
            now,
        );
        // Neither of these is tiny_memory, and cc_retention owns the
        // transcript.
        touch(
            &projects.join("c").join("memory").join("z.md"),
            60 * DAY,
            now,
        );
        touch(&projects.join("a").join("s.jsonl"), 60 * DAY, now);
        let got = scan_swept_in(cfg, now, Some(now - 30 * DAY));
        let tm = got.dirs.iter().find(|d| d.id == "tinyMemory").unwrap();
        assert_eq!((tm.entries, tm.already_deletable), (2, 1));
        assert!(!got.scan_incomplete);
    }
}
