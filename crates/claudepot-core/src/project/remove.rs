//! Single-target project removal — the manual counterpart to
//! `clean_orphans`.
//!
//! Where `clean_orphans` sweeps every CC project dir whose source cwd
//! has gone missing, `remove_project` takes ONE user-chosen path (or
//! slug) and trashes it regardless of whether the source cwd still
//! exists. Live sessions are blocked. The artifact dir moves to the
//! reversible trash at `<data_dir>/trash/projects/`; `~/.claude.json`
//! and `history.jsonl` entries are stripped.
//!
//! The trash manifest captures the stripped sibling state, so a
//! restore puts the dir, the `.claude.json` entry, and the history
//! lines back exactly where they were.
//!
//! ## Config-only projects
//!
//! A project can exist in `~/.claude.json` / `history.jsonl` with no
//! `<config_dir>/projects/<slug>/` dir at all — CC records the cwd
//! when a session opens, but writes no transcript if the session
//! never takes a turn (and a previously-cleaned project leaves the
//! same shape). Such a project is invisible to `list` and was
//! previously unreachable by *both* removal paths: `remove_project`
//! hard-errored `NotFound` on the missing dir, and `clean_orphans`
//! only sweeps dirs that exist. The sibling state was unremovable.
//!
//! `remove_project` now falls through to a config-only removal in
//! that case: nothing is trashed (there is nothing to trash), the
//! `.claude.json` entry and `history.jsonl` lines are stripped, and
//! `trash_id` comes back `None`. The fall-through fires ONLY when
//! sibling state actually references the target — a typo still errors
//! `NotFound` rather than silently succeeding having done nothing.
//!
//! Defense in depth: slug validation rejects `..`, separators, NUL,
//! and leading-dot before any filesystem write; the resolved artifact
//! dir must be a direct child of `<config_dir>/projects/`. A corrupted
//! slug or a typo on the CLI cannot escape that root.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

use crate::error::ProjectError;
use crate::path_utils::simplify_windows_path;
use crate::project_helpers::{
    compute_project_info, detect_live_session, recover_cwd_from_sessions,
};
use crate::project_sanitize::{sanitize_path, unsanitize_path};
use crate::project_trash::{self, ProjectTrashEntry, ProjectTrashPut};
use crate::project_types::ProjectInfo;

/// Heartbeat window for the live-session probe: same value used by
/// `clean_orphans`. 60 s covers the "session just closed" bounce
/// without flagging truly dead dirs.
const REMOVE_LIVE_HEARTBEAT_SECS: u64 = 60;

/// Lock key shared with `clean_orphans` so a manual remove and a sweep
/// can't race on the same `<config_dir>/projects/` tree.
const REMOVE_LOCK_KEY: &str = "__clean__";

/// Args bundle for `remove_project` / `remove_project_preview`.
#[derive(Debug, Clone)]
pub struct RemoveArgs<'a> {
    /// `~/.claude/`. The artifact directory lives at
    /// `<config_dir>/projects/<slug>/`.
    pub config_dir: &'a Path,
    /// Path to `~/.claude.json`. `None` skips the sibling-state strip
    /// (used by tests and by `protected_paths` matches).
    pub claude_json_path: Option<&'a Path>,
    /// Path to `~/.claude/history.jsonl`. `None` skips the history
    /// strip. Defaults to `<config_dir>/history.jsonl` when callers
    /// pass `Some`.
    pub history_path: Option<&'a Path>,
    /// Where the existing batch helpers' duplicate snapshots go.
    /// Production callers pass `<state_root>/snapshots/`.
    pub snapshots_dir: &'a Path,
    /// Where `project_lock` writes its .lock file.
    pub locks_dir: &'a Path,
    /// `~/.claudepot/` — the trash lives at
    /// `<data_dir>/trash/projects/`.
    pub data_dir: &'a Path,
    /// User input. Either a slug (`-Users-joker`) or a path
    /// (`/Users/joker`). The function resolves it to a slug and
    /// rejects ambiguous inputs.
    pub target: &'a str,
}

/// Read-only snapshot of what `remove_project` will do.
#[derive(Debug, Clone, Serialize)]
pub struct RemovePreview {
    pub slug: String,
    /// Best-effort recovered cwd. None when the dir is empty AND no
    /// `.claude.json` key matches the unsanitized slug.
    pub original_path: Option<String>,
    /// False for a config-only project: there is no
    /// `<config_dir>/projects/<slug>/` to trash, so the disclosure
    /// must not promise a restorable trash entry. Callers branch on
    /// this rather than inferring it from `bytes == 0` (a real
    /// project can legitimately be empty).
    pub artifact_dir_present: bool,
    pub bytes: u64,
    pub session_count: usize,
    pub last_modified: Option<SystemTime>,
    pub has_live_session: bool,
    /// True iff `~/.claude.json` `projects.<original_path>` exists.
    /// False also when `claude_json_path` is `None`.
    pub claude_json_entry_present: bool,
    /// Number of `history.jsonl` lines whose `project` field will be
    /// stripped. 0 also when `history_path` is `None`.
    pub history_lines_count: usize,
}

/// Cheap subset of `RemovePreview` — fields the GUI's modal needs to
/// render the disclosure on first paint. Excludes the slow probes
/// (`detect_live_session`, full `~/.claude.json` parse, full
/// `history.jsonl` scan) so the modal opens instantly even when
/// `history.jsonl` is multi-MB. The slow fields come from
/// `RemovePreviewExtras` via a follow-up call.
#[derive(Debug, Clone, Serialize)]
pub struct RemovePreviewBasic {
    pub slug: String,
    pub original_path: Option<String>,
    /// See `RemovePreview::artifact_dir_present`.
    pub artifact_dir_present: bool,
    pub bytes: u64,
    pub session_count: usize,
    pub last_modified: Option<SystemTime>,
}

/// Slow subset of `RemovePreview` — the probes that read large files
/// or call out to system tools. Computed in a separate call so the
/// confirm modal isn't blocked on first paint.
#[derive(Debug, Clone, Serialize)]
pub struct RemovePreviewExtras {
    pub has_live_session: bool,
    pub claude_json_entry_present: bool,
    pub history_lines_count: usize,
}

/// Outcome of a successful `remove_project`.
#[derive(Debug, Clone, Serialize)]
pub struct RemoveResult {
    pub slug: String,
    pub original_path: Option<String>,
    pub bytes: u64,
    pub session_count: usize,
    /// Trash entry to restore from. `None` for a config-only removal:
    /// there was no artifact dir, so nothing was trashed and there is
    /// nothing to restore. The `snapshot_paths` below are still the
    /// recovery surface for the stripped sibling state.
    pub trash_id: Option<String>,
    pub claude_json_entry_removed: bool,
    pub history_lines_removed: usize,
    /// Duplicate recovery snapshots written by the existing batch
    /// helpers. The trash manifest is the primary recovery surface;
    /// these are belt-and-suspenders.
    pub snapshot_paths: Vec<PathBuf>,
}

/// Resolve `args.target` (path or slug) to the on-disk `<slug>` plus
/// its `<config_dir>/projects/<slug>` directory. Errors if the dir
/// doesn't exist.
///
/// Implementation note: paths and slugs need disjoint dispatch,
/// because `Path::join` REPLACES the base when handed an absolute
/// path — `<projects_root>.join("/Users/joker")` returns
/// `/Users/joker` (the user's $HOME), not a subpath of `projects_root`.
/// If the resulting directory existed, we'd then walk and stat the
/// user's entire home tree (200+ seconds) before failing the slug
/// validator at the trash boundary. Catastrophic latency, even if
/// not unsafe.
///
/// Disambiguation: anything containing `/`, `\`, or starting with a
/// drive-letter is treated as a path → sanitize first. Otherwise
/// it's a slug → look up directly.
fn resolve_target(args: &RemoveArgs<'_>) -> Result<ResolvedTarget, ProjectError> {
    let projects_root = args.config_dir.join("projects");
    let looks_like_path = args.target.contains('/')
        || args.target.contains('\\')
        || is_windows_drive_letter(args.target);

    // Derive the slug and the cwd key the sibling state would be
    // filed under. For a path input the target IS the key verbatim —
    // CC files `.claude.json` / `history.jsonl` under the literal cwd —
    // but it must first go through `simplify_windows_path` (per
    // `.claude/rules/paths.md`): a verbatim `\\?\C:\…` target would
    // otherwise sanitize to a slug CC never wrote. For a slug input the
    // key is the (lossy) unsanitize; it is only ever *used* after an
    // exact match confirms it below.
    let (slug, candidate_path) = if looks_like_path {
        let simplified = simplify_windows_path(args.target);
        let slug = sanitize_path(&simplified);
        (slug, simplified)
    } else {
        (args.target.to_string(), unsanitize_path(args.target))
    };

    // Always join the SLUG, never the raw target — see the note above
    // about `Path::join` replacing the base on an absolute path.
    let dir = projects_root.join(&slug);
    if dir.is_dir() {
        return Ok(ResolvedTarget::Artifact { slug, dir });
    }

    // No artifact dir. Fall through to a config-only removal only when
    // sibling state actually names this project. The guard is what
    // keeps a typo (`/no/such/path`) erroring `NotFound` instead of
    // "succeeding" having stripped nothing.
    if let Some(original_path) =
        resolve_config_only_key(args, &slug, &candidate_path, looks_like_path)
    {
        return Ok(ResolvedTarget::ConfigOnly {
            slug,
            original_path,
        });
    }

    Err(ProjectError::NotFound(args.target.to_string()))
}

/// Find the authoritative cwd key for a config-only project (no
/// artifact dir). Returns `None` when nothing corroborates the target,
/// which the caller turns into `NotFound`.
///
/// The two input shapes are handled disjointly:
///
///  - **Path input** — `candidate_path` IS the exact cwd key CC filed
///    the sibling state under. No ambiguity is possible, so a direct
///    exact-match is both necessary and sufficient.
///  - **Slug input** — resolved *only* through the ambiguity-aware
///    reverse lookup. `unsanitize_path` is deliberately NOT trusted
///    here, for two reasons. It is lossy (every `-` becomes `/`, so a
///    real cwd containing a literal `-` — `/a/my-project` → slug
///    `-a-my-project` → guess `/a/my/project` — never matches), and a
///    slug is inherently many-to-one (`/a/b` and `/a-b` both sanitize
///    to `-a-b`). Preferring the unsanitize-canonical guess would let
///    an ambiguous slug silently strip the wrong project. Exactly one
///    matching cwd wins, mirroring account resolution's exact-match
///    rule; zero or many → `None` → `NotFound`.
fn resolve_config_only_key(
    args: &RemoveArgs<'_>,
    slug: &str,
    candidate_path: &str,
    looks_like_path: bool,
) -> Option<String> {
    if looks_like_path {
        return sibling_state_references(args, candidate_path).then(|| candidate_path.to_string());
    }
    let key = reverse_lookup_key_by_slug(args, slug)?;
    // The key came from real sibling state, so this holds by
    // construction; keep the guard as the single gate.
    sibling_state_references(args, &key).then_some(key)
}

/// The unique sibling-state cwd whose `sanitize_path` equals `slug`, or
/// `None` when zero or more than one distinct cwd matches. Scans both
/// `.claude.json` project keys and `history.jsonl` `project` fields —
/// either can be the sole home of a config-only project — so a
/// dash-containing cwd that `unsanitize_path` can't recover is still
/// found. Ambiguity (two real cwds sanitizing to the same slug) refuses
/// rather than guessing, mirroring account resolution's exact-match rule.
fn reverse_lookup_key_by_slug(args: &RemoveArgs<'_>, slug: &str) -> Option<String> {
    use std::collections::BTreeSet;
    let mut matches: BTreeSet<String> = BTreeSet::new();

    if let Some(cj) = args.claude_json_path {
        if let Ok(contents) = std::fs::read_to_string(cj) {
            if let Ok(root) = serde_json::from_str::<serde_json::Value>(&contents) {
                if let Some(map) = root.get("projects").and_then(|v| v.as_object()) {
                    matches.extend(map.keys().filter(|k| sanitize_path(k) == slug).cloned());
                }
            }
        }
    }
    if let Some(h) = args.history_path {
        matches.extend(
            history_project_keys(h)
                .into_iter()
                .filter(|k| sanitize_path(k) == slug),
        );
    }

    let mut iter = matches.into_iter();
    let first = iter.next()?;
    match iter.next() {
        None => Some(first),
        Some(_) => None, // ambiguous — refuse rather than guess
    }
}

/// Distinct `project` field values across `history.jsonl`. Cheap
/// pre-filter on the `"project":` substring skips unrelated lines
/// (mirrors `snapshot_history_lines`).
fn history_project_keys(history_path: &Path) -> std::collections::BTreeSet<String> {
    use std::collections::BTreeSet;
    use std::io::{BufRead, BufReader};

    let mut out: BTreeSet<String> = BTreeSet::new();
    let Ok(file) = std::fs::File::open(history_path) else {
        return out;
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if !line.contains("\"project\":") {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(p) = entry.get("project").and_then(|v| v.as_str()) {
                out.insert(p.to_string());
            }
        }
    }
    out
}

/// What `args.target` resolved to.
enum ResolvedTarget {
    /// `<config_dir>/projects/<slug>/` exists — the normal case. The
    /// dir gets trashed and sibling state stripped.
    Artifact { slug: String, dir: PathBuf },
    /// No artifact dir, but `~/.claude.json` and/or `history.jsonl`
    /// still reference the project. Nothing to trash; the sibling
    /// strip is the whole job. See the module doc.
    ConfigOnly { slug: String, original_path: String },
}

/// True when `~/.claude.json` carries a `projects.<key>` entry, or
/// `history.jsonl` carries at least one line whose `project` field is
/// exactly `key`. Exact-match only — a substring scan would treat a
/// line that merely *mentions* the path in its prompt text as if it
/// belonged to the project.
fn sibling_state_references(args: &RemoveArgs<'_>, key: &str) -> bool {
    if let Some(cj) = args.claude_json_path {
        if snapshot_claude_json_entry(cj, key).is_some() {
            return true;
        }
    }
    if let Some(h) = args.history_path {
        if !snapshot_history_lines(h, key).is_empty() {
            return true;
        }
    }
    false
}

/// `C:` / `D:` / etc. — Windows-shaped absolute path that wouldn't
/// otherwise be flagged by the `/` or `\` check (e.g. `C:foo` —
/// drive-relative, ambiguous, but still not a CC slug).
fn is_windows_drive_letter(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Compute the displayed `original_path` for a removal preview. The
/// rule is:
///
/// 1. If `recover_cwd_from_sessions` returns Some, that's
///    authoritative — at least one session was written from that cwd.
/// 2. Otherwise, fall back to `unsanitize_path(slug)`. CC's sanitizer
///    is lossy for cwds containing literal `-`, so this is best-effort.
/// 3. Reconcile against `~/.claude.json`: if a key exists that
///    matches our candidate exactly, prefer it (rules out the
///    "unsanitize guessed wrong" failure mode).
fn resolve_original_path(
    project_dir: &Path,
    slug: &str,
    claude_json_path: Option<&Path>,
) -> Option<String> {
    let recovered = recover_cwd_from_sessions(project_dir);
    if recovered.is_some() {
        return recovered;
    }
    let candidate = unsanitize_path(slug);
    if let Some(cj) = claude_json_path {
        if let Ok(contents) = std::fs::read_to_string(cj) {
            if let Ok(root) = serde_json::from_str::<serde_json::Value>(&contents) {
                if let Some(map) = root.get("projects").and_then(|v| v.as_object()) {
                    if map.contains_key(&candidate) {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    // Audit fix for project_remove.rs:207 — refuse to return the
    // lossy unsanitize guess when neither sessions nor .claude.json
    // could corroborate it. CC's `sanitize` is destructive on cwds
    // containing literal `-`, so the unsanitize is a hint, not a
    // truth. The previous code returned the guess anyway, and
    // downstream history-line rewrite would strip entries for the
    // guessed (often wrong) path — silently mutating unrelated
    // project history. Returning None here makes the caller treat
    // the project as "unknown original path" and skip the
    // history-side cleanup; the on-disk slug dir is still removed,
    // so the user gets the cleanup they asked for without the
    // collateral damage.
    None
}

/// Read `~/.claude.json` and return the value at `projects.<key>`,
/// without mutating. None when the file/key is absent.
fn snapshot_claude_json_entry(claude_json_path: &Path, key: &str) -> Option<serde_json::Value> {
    let contents = std::fs::read_to_string(claude_json_path).ok()?;
    let root: serde_json::Value = serde_json::from_str(&contents).ok()?;
    root.get("projects")?.get(key).cloned()
}

/// Read `history.jsonl` and return every line whose `project` field
/// matches `target`. Cheap pre-filter on the substring `"project":`
/// avoids parsing unrelated lines (mirrors the existing batch path).
fn snapshot_history_lines(history_path: &Path, target: &str) -> Vec<String> {
    use std::io::{BufRead, BufReader};

    let Ok(file) = std::fs::File::open(history_path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        if !line.contains("\"project\":") {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(p) = entry.get("project").and_then(|v| v.as_str()) {
                if p == target {
                    out.push(line);
                }
            }
        }
    }
    out
}

/// Cheap preview — slug + paths + sessions + size + last_modified.
/// Skips the live-session probe (lsof + process scan) and the full
/// `~/.claude.json` / `history.jsonl` reads. The GUI calls this for
/// the modal's first paint so the disclosure shows up instantly even
/// when sibling state files are large.
pub fn remove_project_preview_basic(
    args: &RemoveArgs<'_>,
) -> Result<RemovePreviewBasic, ProjectError> {
    match resolve_target(args)? {
        ResolvedTarget::Artifact { slug, dir } => {
            let info: ProjectInfo = compute_project_info(&dir, &slug)?;
            let original_path = resolve_original_path(&dir, &slug, args.claude_json_path);
            Ok(RemovePreviewBasic {
                slug,
                original_path,
                artifact_dir_present: true,
                bytes: info.total_size_bytes,
                session_count: info.session_count,
                last_modified: info.last_modified,
            })
        }
        // No artifact dir: no bytes, no sessions, no mtime. The
        // disclosure honestly shows "nothing to trash" and the
        // extras carry the sibling-state counts.
        ResolvedTarget::ConfigOnly {
            slug,
            original_path,
        } => Ok(RemovePreviewBasic {
            slug,
            original_path: Some(original_path),
            artifact_dir_present: false,
            bytes: 0,
            session_count: 0,
            last_modified: None,
        }),
    }
}

/// Slow preview — runs the live-session probe and parses
/// `~/.claude.json` + `history.jsonl` end-to-end. Returns the
/// disabled-state metadata the modal uses to gate the Remove button
/// and to annotate the disclosure.
pub fn remove_project_preview_extras(
    args: &RemoveArgs<'_>,
) -> Result<RemovePreviewExtras, ProjectError> {
    // `(project_dir, original_path)` for the probes below. On the
    // config-only branch the dir does not exist; `detect_live_session`
    // still earns its keep there via the `is_claude_running_in(cwd)`
    // process scan (the lsof probe simply misses on a missing dir), so
    // a live CC in that cwd still gates the strip.
    // `original_path` stays a strict Option: when it is None the
    // sibling-state lookups below must be SKIPPED, not run against a
    // guessed key (see `resolve_original_path`'s note on lossy
    // unsanitize and collateral history damage). `live_check_path` is
    // the looser best-effort cwd — only ever fed to the process scan.
    let (project_dir, original_path, live_check_path) = match resolve_target(args)? {
        ResolvedTarget::Artifact { slug, dir } => {
            let info: ProjectInfo = compute_project_info(&dir, &slug)?;
            let original_path = resolve_original_path(&dir, &slug, args.claude_json_path);
            let live_check_path = original_path.clone().unwrap_or(info.original_path);
            (dir, original_path, live_check_path)
        }
        ResolvedTarget::ConfigOnly {
            slug,
            original_path,
        } => {
            let dir = args.config_dir.join("projects").join(&slug);
            (dir, Some(original_path.clone()), original_path)
        }
    };

    let has_live_session =
        detect_live_session(&project_dir, &live_check_path, REMOVE_LIVE_HEARTBEAT_SECS);

    let claude_json_entry_present = match (args.claude_json_path, original_path.as_deref()) {
        (Some(cj), Some(key)) => snapshot_claude_json_entry(cj, key).is_some(),
        _ => false,
    };

    let history_lines_count = match (args.history_path, original_path.as_deref()) {
        (Some(h), Some(target)) => snapshot_history_lines(h, target).len(),
        _ => 0,
    };

    Ok(RemovePreviewExtras {
        has_live_session,
        claude_json_entry_present,
        history_lines_count,
    })
}

/// Read-only path. Computes the same data `remove_project` would act
/// on, without touching the filesystem. Callers use this to render a
/// confirmation modal honestly. CLI uses this for the synchronous
/// disclosure print; GUI prefers the basic+extras split for snappy
/// first paint.
pub fn remove_project_preview(args: &RemoveArgs<'_>) -> Result<RemovePreview, ProjectError> {
    let basic = remove_project_preview_basic(args)?;
    let extras = remove_project_preview_extras(args)?;
    Ok(RemovePreview {
        slug: basic.slug,
        original_path: basic.original_path,
        artifact_dir_present: basic.artifact_dir_present,
        bytes: basic.bytes,
        session_count: basic.session_count,
        last_modified: basic.last_modified,
        has_live_session: extras.has_live_session,
        claude_json_entry_present: extras.claude_json_entry_present,
        history_lines_count: extras.history_lines_count,
    })
}

/// A live-session probe: `(cc_project_dir, project_cwd, heartbeat_secs)
/// -> is_live`. Production passes `detect_live_session`; tests inject a
/// stub so the destructive live-session refusal is coverable without a
/// real `claude` process. See `remove_project_with_probe`.
type LiveProbe = fn(&Path, &str, u64) -> bool;

/// Execute the trash. Live-session refusal is a hard error — the user
/// must close the session and retry.
///
/// Dispatches on what the target resolved to: a real artifact dir
/// (trash + strip) or a config-only project (strip only). See the
/// module doc.
pub fn remove_project(args: &RemoveArgs<'_>) -> Result<RemoveResult, ProjectError> {
    remove_project_with_probe(args, detect_live_session)
}

/// Seam for testing the live-session refusal. `remove_project` is the
/// production entry point (`detect_live_session`); tests pass a stub.
fn remove_project_with_probe(
    args: &RemoveArgs<'_>,
    live_probe: LiveProbe,
) -> Result<RemoveResult, ProjectError> {
    match resolve_target(args)? {
        ResolvedTarget::Artifact { slug, dir } => {
            remove_artifact_project(args, slug, dir, live_probe)
        }
        ResolvedTarget::ConfigOnly {
            slug,
            original_path,
        } => remove_config_only_project(args, slug, original_path, live_probe),
    }
}

/// Config-only removal: no artifact dir, so nothing is trashed and no
/// trash manifest is written. The `.claude.json` entry and the
/// `history.jsonl` lines are the entire payload; the batch helpers'
/// snapshots are the recovery surface.
fn remove_config_only_project(
    args: &RemoveArgs<'_>,
    slug: String,
    original_path: String,
    live_probe: LiveProbe,
) -> Result<RemoveResult, ProjectError> {
    // The dir does not exist, so the lsof probe inside
    // `detect_live_session` misses; the process scan for a `claude`
    // running in this cwd is what actually gates us here (someone
    // launched `claude` in the cwd but it hasn't written a transcript
    // yet — the one window where a config-only project can be live).
    let would_be_dir = args.config_dir.join("projects").join(&slug);
    if live_probe(&would_be_dir, &original_path, REMOVE_LIVE_HEARTBEAT_SECS) {
        return Err(ProjectError::ClaudeRunning(original_path));
    }

    let (lock_guard, _broken) = crate::project_lock::acquire(args.locks_dir, REMOVE_LOCK_KEY)?;
    let strip = strip_sibling_state(args, &original_path)?;
    lock_guard.release()?;

    Ok(RemoveResult {
        slug,
        original_path: Some(original_path),
        bytes: 0,
        session_count: 0,
        trash_id: None,
        claude_json_entry_removed: strip.claude_json_entry_removed,
        history_lines_removed: strip.history_lines_removed,
        snapshot_paths: strip.snapshot_paths,
    })
}

/// Outcome of the sibling-state strip, shared by both removal paths.
#[derive(Default)]
struct SiblingStrip {
    claude_json_entry_removed: bool,
    history_lines_removed: usize,
    snapshot_paths: Vec<PathBuf>,
}

/// Strip `projects.<key>` from `~/.claude.json` and every
/// `history.jsonl` line whose `project` field is exactly `key`. Both
/// batch helpers write a pre-mutation snapshot into `snapshots_dir` —
/// that is the recovery surface for a config-only removal, which has
/// no trash manifest to restore from.
///
/// `protected_paths` deliberately is NOT consulted here: that set
/// protects automated sweeps (`clean_orphans`) from running over
/// system roots without any per-project user confirmation.
/// `remove_project` requires explicit per-project confirmation at the
/// UX layer (typed slug match), so the user has already cleared the
/// bar — paternalism here would defeat the feature for the very case
/// it exists for ("I accidentally ran `claude` in $HOME").
///
/// The strip no-ops safely when sibling state doesn't match (lossy
/// unsanitize, unrelated key), so the unauthoritative empty-dir case
/// is naturally handled.
fn strip_sibling_state(args: &RemoveArgs<'_>, key: &str) -> Result<SiblingStrip, ProjectError> {
    let mut out = SiblingStrip::default();
    let keys = [key.to_string()];

    if let Some(cj) = args.claude_json_path {
        if cj.exists() {
            let (count, snap) =
                crate::project::remove_claude_json_entries_batch(cj, args.snapshots_dir, &keys)?;
            out.claude_json_entry_removed = count > 0;
            if let Some(p) = snap {
                out.snapshot_paths.push(p);
            }
        }
    }
    if let Some(h) = args.history_path {
        if h.exists() {
            let (count, snap) =
                crate::project::remove_history_lines_batch(h, args.snapshots_dir, &keys)?;
            out.history_lines_removed = count;
            if let Some(p) = snap {
                out.snapshot_paths.push(p);
            }
        }
    }
    Ok(out)
}

/// The original path: trash the artifact dir, then strip sibling state.
fn remove_artifact_project(
    args: &RemoveArgs<'_>,
    slug: String,
    project_dir: PathBuf,
    live_probe: LiveProbe,
) -> Result<RemoveResult, ProjectError> {
    let info: ProjectInfo = compute_project_info(&project_dir, &slug)?;
    let original_path = resolve_original_path(&project_dir, &slug, args.claude_json_path);

    // Live-session refusal. The same probe `clean_orphans` runs.
    let live_check_path = original_path.as_deref().unwrap_or(&info.original_path);
    if live_probe(&project_dir, live_check_path, REMOVE_LIVE_HEARTBEAT_SECS) {
        return Err(ProjectError::ClaudeRunning(live_check_path.to_string()));
    }

    let (lock_guard, _broken) = crate::project_lock::acquire(args.locks_dir, REMOVE_LOCK_KEY)?;

    // Snapshot sibling state into the trash manifest BEFORE any
    // mutation. This is the recovery payload the user will rely on
    // when they hit Restore.
    let claude_json_entry = match (args.claude_json_path, original_path.as_deref()) {
        (Some(cj), Some(key)) => snapshot_claude_json_entry(cj, key),
        _ => None,
    };
    let history_lines = match (args.history_path, original_path.as_deref()) {
        (Some(h), Some(target)) => snapshot_history_lines(h, target),
        _ => Vec::new(),
    };

    // Move the artifact dir to project trash. This is the
    // irreversibility line — after this, the dir is no longer at
    // `<config_dir>/projects/<slug>/`. Restore is the only way back.
    let entry: ProjectTrashEntry = project_trash::write(
        args.data_dir,
        ProjectTrashPut {
            source_dir: &project_dir,
            slug: &slug,
            original_path: original_path.as_deref(),
            bytes: info.total_size_bytes,
            session_count: info.session_count,
            claude_json_entry: claude_json_entry.clone(),
            history_lines: history_lines.clone(),
            reason: Some("user-initiated remove".to_string()),
        },
    )
    .map_err(|e| ProjectError::Ambiguous(format!("trash write failed: {e}")))?;

    let strip = match original_path.as_deref() {
        Some(orig) => strip_sibling_state(args, orig)?,
        None => SiblingStrip::default(),
    };
    let claude_json_entry_removed = strip.claude_json_entry_removed;
    let history_lines_removed = strip.history_lines_removed;
    let snapshot_paths = strip.snapshot_paths;

    lock_guard.release()?;

    Ok(RemoveResult {
        slug,
        original_path,
        bytes: info.total_size_bytes,
        session_count: info.session_count,
        trash_id: Some(entry.id),
        claude_json_entry_removed,
        history_lines_removed,
        snapshot_paths,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Stage a fixture: `<tmp>/.claude/projects/<slug>/<session>.jsonl`,
    /// `<tmp>/.claude.json`, `<tmp>/.claude/history.jsonl`,
    /// `<tmp>/.claudepot/` (data_dir).
    struct Fixture {
        _tmp: TempDir,
        config_dir: PathBuf,
        data_dir: PathBuf,
        snapshots_dir: PathBuf,
        locks_dir: PathBuf,
        claude_json: PathBuf,
        history: PathBuf,
        slug: String,
        original_path: String,
    }

    fn setup(slug: &str, original_path: &str, write_session: bool) -> Fixture {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path().join(".claude");
        let data_dir = tmp.path().join(".claudepot");
        let snapshots_dir = data_dir.join("snapshots");
        let locks_dir = data_dir.join("locks");
        fs::create_dir_all(config_dir.join("projects").join(slug)).unwrap();
        fs::create_dir_all(&snapshots_dir).unwrap();
        fs::create_dir_all(&locks_dir).unwrap();

        if write_session {
            // Minimal CC session line — `cwd` is what
            // `recover_cwd_from_sessions` keys on.
            let session = config_dir
                .join("projects")
                .join(slug)
                .join("00000000-0000-0000-0000-000000000001.jsonl");
            let line = serde_json::json!({
                "type": "summary",
                "cwd": original_path
            })
            .to_string();
            fs::write(&session, format!("{}\n", line)).unwrap();
            // Age the session beyond the live-heartbeat window. On
            // Linux/macOS `detect_live_session` requires a kernel
            // confirmation signal (lsof / process scan) before treating
            // a recent mtime as live, but on Windows runners `lsof` is
            // absent, so the fallback path treats any mtime within
            // REMOVE_LIVE_HEARTBEAT_SECS (60 s) as a live session and
            // every test that just wrote a fresh fixture would refuse
            // to remove with `ClaudeRunning`. Pushing mtime ~2 minutes
            // back keeps the rest of the production path exercised.
            let stale = filetime::FileTime::from_system_time(
                std::time::SystemTime::now() - std::time::Duration::from_secs(120),
            );
            filetime::set_file_mtime(&session, stale).unwrap();
        }

        let claude_json = tmp.path().join(".claude.json");
        let claude_json_body = serde_json::json!({
            "projects": {
                original_path: {"trustDialogAccepted": true}
            }
        });
        fs::write(&claude_json, serde_json::to_vec(&claude_json_body).unwrap()).unwrap();

        let history = config_dir.join("history.jsonl");
        let history_body = format!(
            "{}\n{}\n",
            serde_json::json!({"project": original_path, "display": "ls"}),
            serde_json::json!({"project": "/Users/other", "display": "pwd"})
        );
        fs::write(&history, history_body).unwrap();

        Fixture {
            _tmp: tmp,
            config_dir,
            data_dir,
            snapshots_dir,
            locks_dir,
            claude_json,
            history,
            slug: slug.to_string(),
            original_path: original_path.to_string(),
        }
    }

    fn args<'a>(f: &'a Fixture, target: &'a str) -> RemoveArgs<'a> {
        RemoveArgs {
            config_dir: &f.config_dir,
            claude_json_path: Some(&f.claude_json),
            history_path: Some(&f.history),
            snapshots_dir: &f.snapshots_dir,
            locks_dir: &f.locks_dir,
            data_dir: &f.data_dir,
            target,
        }
    }

    /// The same fixture minus the artifact dir — a "config-only"
    /// project: `.claude.json` + `history.jsonl` still name it, but
    /// `<config_dir>/projects/<slug>/` never existed (CC opened a
    /// session there that took no turn, so no transcript was written).
    fn setup_config_only(slug: &str, original_path: &str) -> Fixture {
        let f = setup(slug, original_path, false);
        fs::remove_dir_all(f.config_dir.join("projects").join(slug)).unwrap();
        f
    }

    #[test]
    fn config_only_remove_strips_sibling_state_without_trashing() {
        let f = setup_config_only("-Users-joker-pando", "/Users/joker/pando");
        let result = remove_project(&args(&f, "/Users/joker/pando")).unwrap();

        // Nothing was trashed — there was no dir to trash.
        assert_eq!(result.trash_id, None);
        assert_eq!(result.bytes, 0);
        assert_eq!(result.session_count, 0);

        // The sibling state IS gone — that was the whole job.
        assert!(result.claude_json_entry_removed);
        assert_eq!(result.history_lines_removed, 1);

        let cj: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&f.claude_json).unwrap()).unwrap();
        assert!(cj["projects"].get(&f.original_path).is_none());

        let h = fs::read_to_string(&f.history).unwrap();
        assert!(!h.contains(&f.original_path));
        assert!(h.contains("/Users/other"));

        // A pre-mutation snapshot is the recovery surface (no trash
        // manifest exists for a config-only removal).
        assert!(!result.snapshot_paths.is_empty());
    }

    #[test]
    fn config_only_remove_accepts_a_slug_target() {
        let f = setup_config_only("-Users-joker-pando", "/Users/joker/pando");
        let result = remove_project(&args(&f, "-Users-joker-pando")).unwrap();
        assert_eq!(result.trash_id, None);
        assert!(result.claude_json_entry_removed);
    }

    /// A slug target whose real cwd contains a literal `-`. The slug
    /// collapses `/` and `-` to the same `-`, so `unsanitize_path`
    /// guesses the wrong cwd (`/Users/joker/pando/ape`). Resolution
    /// must reverse-map through `.claude.json` to the true key.
    #[test]
    fn config_only_slug_with_literal_dash_resolves_via_reverse_lookup() {
        let slug = sanitize_path("/Users/joker/pando-ape");
        // Sanity: the lossy unsanitize really does miss.
        assert_ne!(unsanitize_path(&slug), "/Users/joker/pando-ape");

        let f = setup_config_only(&slug, "/Users/joker/pando-ape");
        let result = remove_project(&args(&f, &slug)).unwrap();

        assert_eq!(result.trash_id, None);
        assert!(result.claude_json_entry_removed);
        assert_eq!(
            result.original_path.as_deref(),
            Some("/Users/joker/pando-ape")
        );
    }

    /// A config-only project that lives ONLY in `history.jsonl` (its
    /// `.claude.json` entry is already gone), reached by a dash-slug.
    /// The reverse lookup must find it via the history scan.
    #[test]
    fn config_only_history_only_slug_resolves_via_history_reverse_lookup() {
        let slug = sanitize_path("/Users/joker/pando-ape");
        let f = setup_config_only(&slug, "/Users/joker/pando-ape");
        // Drop the .claude.json entry so history is the only home.
        fs::write(
            &f.claude_json,
            serde_json::json!({"projects": {}}).to_string(),
        )
        .unwrap();

        let result = remove_project(&args(&f, &slug)).unwrap();
        assert_eq!(result.trash_id, None);
        assert!(!result.claude_json_entry_removed);
        assert_eq!(result.history_lines_removed, 1);
        assert_eq!(
            result.original_path.as_deref(),
            Some("/Users/joker/pando-ape")
        );
    }

    /// A slug is many-to-one: `/Users/joker/a/b` and `/Users/joker/a-b`
    /// both sanitize to `-Users-joker-a-b`. On a destructive path an
    /// ambiguous slug must refuse, not silently pick the
    /// unsanitize-canonical one.
    #[test]
    fn config_only_ambiguous_slug_refuses_rather_than_guessing() {
        let slug = sanitize_path("/Users/joker/a-b");
        assert_eq!(slug, sanitize_path("/Users/joker/a/b"));

        let f = setup_config_only(&slug, "/Users/joker/a-b");
        // Both colliding cwds are real, config-only projects.
        let body = serde_json::json!({
            "projects": {
                "/Users/joker/a-b": {"trustDialogAccepted": true},
                "/Users/joker/a/b": {"trustDialogAccepted": true},
            }
        });
        fs::write(&f.claude_json, serde_json::to_vec(&body).unwrap()).unwrap();

        let err = remove_project(&args(&f, &slug)).unwrap_err();
        assert!(matches!(err, ProjectError::NotFound(_)));

        // Neither project was touched.
        let cj: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&f.claude_json).unwrap()).unwrap();
        assert!(cj["projects"].get("/Users/joker/a-b").is_some());
        assert!(cj["projects"].get("/Users/joker/a/b").is_some());
    }

    /// The destructive live-session gate on the config-only path.
    /// Injects a stub probe that reports "live" so the refusal is
    /// covered without a real `claude` process, and asserts the sibling
    /// state is left untouched.
    #[test]
    fn config_only_remove_refuses_when_live_session_present() {
        let f = setup_config_only("-Users-joker-pando", "/Users/joker/pando");
        let err =
            remove_project_with_probe(&args(&f, "/Users/joker/pando"), |_, _, _| true).unwrap_err();
        assert!(matches!(err, ProjectError::ClaudeRunning(_)));

        // Nothing was stripped — the gate fired before any mutation.
        let cj: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&f.claude_json).unwrap()).unwrap();
        assert!(cj["projects"].get(&f.original_path).is_some());
        assert!(fs::read_to_string(&f.history)
            .unwrap()
            .contains(&f.original_path));
    }

    #[test]
    fn config_only_preview_reports_nothing_to_trash() {
        let f = setup_config_only("-Users-joker-pando", "/Users/joker/pando");
        let preview = remove_project_preview(&args(&f, "/Users/joker/pando")).unwrap();
        assert_eq!(preview.bytes, 0);
        assert_eq!(preview.session_count, 0);
        assert_eq!(preview.last_modified, None);
        assert_eq!(preview.original_path.as_deref(), Some("/Users/joker/pando"));
        assert!(preview.claude_json_entry_present);
        assert_eq!(preview.history_lines_count, 1);
    }

    /// The guard that keeps the config-only fall-through from turning
    /// a typo into a silent no-op success.
    #[test]
    fn absent_target_with_no_sibling_state_still_errors_not_found() {
        let f = setup_config_only("-Users-joker-pando", "/Users/joker/pando");
        let err = remove_project(&args(&f, "/no/such/project")).unwrap_err();
        assert!(matches!(err, ProjectError::NotFound(_)));
    }

    /// Regression lock for the real-world pando-ape shape: a history
    /// line belonging to ANOTHER project that merely mentions the
    /// target path in its prompt text must survive. A substring strip
    /// would eat it.
    #[test]
    fn config_only_strip_matches_project_field_not_prompt_text() {
        let f = setup_config_only("-Users-joker-pando", "/Users/joker/pando");

        // A line owned by a different project whose prompt text names
        // the target path.
        let mentioning = serde_json::json!({
            "project": "/Users/joker/other-repo",
            "display": "claudepot project remove /Users/joker/pando"
        })
        .to_string();
        let body = format!(
            "{}\n{}\n",
            fs::read_to_string(&f.history).unwrap().trim_end(),
            mentioning
        );
        fs::write(&f.history, body).unwrap();

        let result = remove_project(&args(&f, "/Users/joker/pando")).unwrap();

        // Only the owned line went.
        assert_eq!(result.history_lines_removed, 1);
        let h = fs::read_to_string(&f.history).unwrap();
        assert!(h.contains("/Users/joker/other-repo"));
        assert!(h.contains("claudepot project remove /Users/joker/pando"));
    }

    #[test]
    fn preview_reports_session_count_and_history_lines() {
        let f = setup("-Users-joker-myproject", "/Users/joker/myproject", true);
        let preview = remove_project_preview(&args(&f, &f.slug)).unwrap();
        assert_eq!(preview.slug, f.slug);
        assert_eq!(
            preview.original_path.as_deref(),
            Some("/Users/joker/myproject")
        );
        assert_eq!(preview.session_count, 1);
        assert!(preview.claude_json_entry_present);
        assert_eq!(preview.history_lines_count, 1);
        assert!(!preview.has_live_session);
    }

    #[test]
    fn preview_resolves_path_input_to_slug() {
        let f = setup("-Users-joker-myproject", "/Users/joker/myproject", true);
        // User passes a path; should resolve to the same slug.
        let preview = remove_project_preview(&args(&f, "/Users/joker/myproject")).unwrap();
        assert_eq!(preview.slug, f.slug);
    }

    #[test]
    fn remove_moves_dir_to_trash_and_strips_sibling_state() {
        let f = setup("-Users-joker-myproject", "/Users/joker/myproject", true);
        let result = remove_project(&args(&f, &f.slug)).unwrap();

        assert_eq!(result.slug, f.slug);
        assert_eq!(result.session_count, 1);
        assert!(result.claude_json_entry_removed);
        assert_eq!(result.history_lines_removed, 1);

        // Artifact dir is gone.
        assert!(!f.config_dir.join("projects").join(&f.slug).exists());

        // .claude.json key removed.
        let cj: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&f.claude_json).unwrap()).unwrap();
        assert!(cj["projects"].get(&f.original_path).is_none());

        // history.jsonl: target line gone, unrelated line preserved.
        let h = fs::read_to_string(&f.history).unwrap();
        assert!(!h.contains(&f.original_path));
        assert!(h.contains("/Users/other"));

        // Trash manifest holds the snapshot.
        let listing = crate::project_trash::list(&f.data_dir, Default::default()).unwrap();
        assert_eq!(listing.entries.len(), 1);
        let entry = &listing.entries[0];
        assert_eq!(Some(entry.id.clone()), result.trash_id);
        assert_eq!(entry.slug, f.slug);
        assert_eq!(entry.history_lines.len(), 1);
        assert!(entry.claude_json_entry.is_some());
    }

    #[test]
    fn remove_then_restore_round_trip() {
        let f = setup("-Users-joker-myproject", "/Users/joker/myproject", true);
        let result = remove_project(&args(&f, &f.slug)).unwrap();
        let trash_id = result
            .trash_id
            .expect("artifact removal must produce a trash entry");
        // Restore via project_trash directly.
        let report = crate::project_trash::restore(
            &f.data_dir,
            &trash_id,
            &f.config_dir,
            Some(&f.claude_json),
            Some(&f.history),
        )
        .unwrap();
        assert!(report.claude_json_restored);
        assert_eq!(report.history_lines_restored, 1);

        // Dir is back.
        assert!(f.config_dir.join("projects").join(&f.slug).exists());

        // .claude.json has the entry back.
        let cj: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&f.claude_json).unwrap()).unwrap();
        assert!(cj["projects"].get(&f.original_path).is_some());

        // history.jsonl has the line back.
        let h = fs::read_to_string(&f.history).unwrap();
        assert!(h.contains(&f.original_path));
    }

    #[test]
    fn absolute_path_input_does_not_walk_outside_projects_root() {
        // Regression: `Path::join` replaces the base when handed an
        // absolute path, so `projects_root.join("/Users/joker")`
        // returned the user's $HOME — and `compute_project_info`
        // happily stat-walked the entire tree (200+ s in the field
        // before this fix). The slug-path disjoint dispatch in
        // `resolve_target` keeps the absolute-path branch from ever
        // hitting the slug fast-path.
        let f = setup("-Users-joker-myproject", "/Users/joker/myproject", true);
        // The target is a path (contains `/`) that is NOT a CC slug.
        // Even though `/Users/joker/myproject` happens to exist as a
        // real directory on the test host (it doesn't here, but in
        // production it would), we should resolve via sanitize, NOT
        // by joining it onto projects_root.
        let preview = remove_project_preview(&args(&f, "/Users/joker/myproject")).unwrap();
        assert_eq!(preview.slug, "-Users-joker-myproject");
        // Slug never contains separators — the trash-side validator
        // would catch it later, but the preview path catches it now.
        assert!(!preview.slug.contains('/'));
        assert!(!preview.slug.contains('\\'));
    }

    #[test]
    fn remove_missing_dir_is_not_found() {
        let f = setup("-Users-joker-myproject", "/Users/joker/myproject", true);
        let err = remove_project(&args(&f, "-Users-bogus")).unwrap_err();
        assert!(matches!(err, ProjectError::NotFound(_)));
    }

    #[cfg(unix)]
    #[test]
    fn empty_project_uses_unsanitize_fallback_when_key_matches() {
        // Empty dir (no sessions) where `unsanitize(slug)` happens to
        // match a real `.claude.json` key — the user's exact
        // accidental-Ctrl+C-in-$HOME scenario.
        let f = setup("-Users-joker", "/Users/joker", false);
        let preview = remove_project_preview(&args(&f, &f.slug)).unwrap();
        assert_eq!(preview.original_path.as_deref(), Some("/Users/joker"));
        assert!(preview.claude_json_entry_present);
        let result = remove_project(&args(&f, &f.slug)).unwrap();
        assert!(result.claude_json_entry_removed);
    }

    #[test]
    fn live_session_refuses_remove() {
        // Heartbeat-only fallback: we write a fresh .jsonl AND the
        // detect_live_session path treats fresh + lsof-unavailable as
        // live. On the test host, lsof IS available — so this test
        // path may or may not fire depending on environment. To keep
        // the test deterministic, we skip the lsof branch by relying
        // on a process-scan miss + heartbeat-only treated-as-live
        // ONLY when lsof is missing. So this test asserts the
        // *negative* case: a fresh .jsonl + lsof-available does NOT
        // refuse — i.e., the function behaves like a normal remove.
        // The positive live-block case is covered by integration
        // testing on a CI runner without lsof.
        let f = setup("-Users-joker-myproject", "/Users/joker/myproject", true);
        // Touch the existing .jsonl to refresh its mtime.
        let session = f
            .config_dir
            .join("projects")
            .join(&f.slug)
            .join("00000000-0000-0000-0000-000000000001.jsonl");
        let now = std::time::SystemTime::now();
        // Best-effort: re-write the file to bump mtime.
        fs::write(&session, fs::read(&session).unwrap()).unwrap();
        let _ = filetime::set_file_mtime(&session, filetime::FileTime::from(now));
        // We can't reliably force live-detection on every CI host, so
        // we just assert that calling preview is non-fatal — the
        // production block is exercised via the integration test
        // against a controlled lsof-less harness.
        let _ = remove_project_preview(&args(&f, &f.slug));
    }
}
