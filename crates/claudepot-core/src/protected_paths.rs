//! User-managed list of paths whose CC artifacts may still be cleaned
//! but whose **sibling state** (`~/.claude.json` projects map,
//! `~/.claude/history.jsonl`) must NOT be rewritten by `clean_orphans`.
//!
//! Two-tier safety:
//!   * The CC artifact dir at `~/.claude/projects/<san>/` is always
//!     scoped to claude's own area — safe to remove regardless.
//!   * Sibling rewrites strip `projects[<orig_path>]` and history lines
//!     keyed on `<orig_path>`. For paths like `/`, `~`, `/Users`, that
//!     wipes real user config the user almost certainly wants to keep.
//!
//! Storage format on disk (deltas, not snapshots):
//! ```json
//! {
//!   "version": 1,
//!   "removed_defaults": ["/tmp"],
//!   "user":             ["/Volumes/work-archive"]
//! }
//! ```
//! Defaults are implicit. A future Claudepot release adding `"/private"`
//! to `DEFAULT_PATHS` reaches existing users automatically; users opt
//! out via `removed_defaults` and add their own via `user`.
//!
//! Persisted at `<data_dir>/protected-paths.json`. `data_dir` is
//! Claudepot's private root (`paths::claudepot_data_dir()`), not the
//! shared `~/.claude/claudepot/` tree — these are user preferences,
//! not CC artifacts.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const STORE_FILENAME: &str = "protected-paths.json";

/// Built-in protected paths. Cross-platform constant: paths that don't
/// apply to the current OS are simply absent from the live filesystem
/// and stay benign in the resolved set.
pub const DEFAULT_PATHS: &[&str] = &[
    // POSIX system roots
    "/",
    "~",
    "/Users",
    "/home",
    "/root",
    "/tmp",
    "/var",
    "/etc",
    "/opt",
    "/usr",
    // Windows drive roots and system dirs (stored verbatim; case is
    // preserved on disk and matched literally — Windows paths in CC's
    // sanitized form already preserve case).
    "C:\\",
    "C:\\Users",
    "C:\\Windows",
    "C:\\Program Files",
    "C:\\Program Files (x86)",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathSource {
    /// Built-in default from `DEFAULT_PATHS` and not in `removed_defaults`.
    Default,
    /// User-added entry.
    User,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProtectedPath {
    pub path: String,
    pub source: PathSource,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    removed_defaults: Vec<String>,
    #[serde(default)]
    user: Vec<String>,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, thiserror::Error)]
pub enum ProtectedPathsError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid JSON in {path}: {err}")]
    InvalidJson {
        path: PathBuf,
        err: serde_json::Error,
    },

    #[error("path is empty")]
    Empty,

    #[error("path must be absolute or start with '~' (got '{0}')")]
    NotAbsolute(String),

    #[error("path is already protected: '{0}'")]
    Duplicate(String),

    #[error("no such protected path: '{0}'")]
    NotFound(String),
}

/// Path of the persisted store file under `data_dir`.
pub fn store_path(data_dir: &Path) -> PathBuf {
    data_dir.join(STORE_FILENAME)
}

fn load_store(data_dir: &Path) -> Result<Store, ProtectedPathsError> {
    let path = store_path(data_dir);
    if !path.exists() {
        return Ok(Store::default());
    }
    let text = fs::read_to_string(&path)?;
    if text.trim().is_empty() {
        return Ok(Store::default());
    }
    serde_json::from_str(&text)
        .map_err(|err| ProtectedPathsError::InvalidJson { path, err })
}

fn save_store(data_dir: &Path, store: &Store) -> Result<(), ProtectedPathsError> {
    fs::create_dir_all(data_dir)?;
    let path = store_path(data_dir);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let json = serde_json::to_string_pretty(store)
        .map_err(|e| ProtectedPathsError::Io(std::io::Error::other(e.to_string())))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(json.as_bytes())?;
    tmp.write_all(b"\n")?;
    tmp.persist(&path).map_err(|e| ProtectedPathsError::Io(e.error))?;
    Ok(())
}

/// Materialized list: defaults (minus `removed_defaults`), then user
/// entries in insertion order. Stable order so the UI can render
/// without sorting.
pub fn list(data_dir: &Path) -> Result<Vec<ProtectedPath>, ProtectedPathsError> {
    let store = load_store(data_dir)?;
    let removed: HashSet<&str> = store.removed_defaults.iter().map(String::as_str).collect();
    let user_set: HashSet<&str> = store.user.iter().map(String::as_str).collect();

    let mut out: Vec<ProtectedPath> = Vec::with_capacity(DEFAULT_PATHS.len() + store.user.len());
    for d in DEFAULT_PATHS {
        if removed.contains(*d) || user_set.contains(*d) {
            continue;
        }
        out.push(ProtectedPath {
            path: (*d).to_string(),
            source: PathSource::Default,
        });
    }
    for u in &store.user {
        out.push(ProtectedPath {
            path: u.clone(),
            source: PathSource::User,
        });
    }
    Ok(out)
}

/// Membership-check set used by `clean_orphans` for the sibling-rewrite
/// guard. Each stored entry contributes itself, and tilde-prefixed
/// entries also contribute their `$HOME`-expanded form.
pub fn resolved_set(data_dir: &Path) -> Result<HashSet<String>, ProtectedPathsError> {
    let mut set = HashSet::new();
    for p in list(data_dir)? {
        if let Some(expanded) = expand_tilde(&p.path) {
            set.insert(expanded);
        }
        set.insert(p.path);
    }
    Ok(set)
}

/// Expand `~` or `~/x` against the current `$HOME`. Returns `None` if
/// the input doesn't start with `~` or if `$HOME` is unavailable.
fn expand_tilde(p: &str) -> Option<String> {
    if p == "~" {
        return dirs::home_dir().map(|h| h.to_string_lossy().to_string());
    }
    if let Some(rest) = p.strip_prefix("~/") {
        return dirs::home_dir().map(|h| h.join(rest).to_string_lossy().to_string());
    }
    None
}

fn validate(input: &str) -> Result<String, ProtectedPathsError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ProtectedPathsError::Empty);
    }
    let is_tilde = trimmed == "~" || trimmed.starts_with("~/");
    let is_absolute = Path::new(trimmed).is_absolute();
    if !is_tilde && !is_absolute {
        return Err(ProtectedPathsError::NotAbsolute(trimmed.to_string()));
    }
    // Normalize trailing slash, except for "/" itself which IS the slash.
    let normalized = if trimmed.len() > 1 && trimmed.ends_with('/') {
        trimmed.trim_end_matches('/').to_string()
    } else {
        trimmed.to_string()
    };
    Ok(normalized)
}

/// Add a user-supplied path. If the path matches a default that was
/// previously removed (`removed_defaults`), we un-remove it instead of
/// duplicating it under `user`.
pub fn add(data_dir: &Path, path: &str) -> Result<ProtectedPath, ProtectedPathsError> {
    let normalized = validate(path)?;
    let mut store = load_store(data_dir)?;

    let is_default = DEFAULT_PATHS.iter().any(|d| *d == normalized.as_str());
    let in_user = store.user.iter().any(|u| u == &normalized);
    let was_removed_default =
        is_default && store.removed_defaults.iter().any(|r| r == &normalized);

    if in_user || (is_default && !was_removed_default) {
        return Err(ProtectedPathsError::Duplicate(normalized));
    }

    if was_removed_default {
        store.removed_defaults.retain(|r| r != &normalized);
        save_store(data_dir, &store)?;
        return Ok(ProtectedPath {
            path: normalized,
            source: PathSource::Default,
        });
    }

    store.user.push(normalized.clone());
    save_store(data_dir, &store)?;
    Ok(ProtectedPath {
        path: normalized,
        source: PathSource::User,
    })
}

/// Remove a protected path. Defaults are tombstoned in
/// `removed_defaults` so a subsequent `reset()` brings them back; user
/// entries are dropped.
pub fn remove(data_dir: &Path, path: &str) -> Result<(), ProtectedPathsError> {
    let normalized = validate(path)?;
    let mut store = load_store(data_dir)?;

    let is_default = DEFAULT_PATHS.iter().any(|d| *d == normalized.as_str());
    let in_user = store.user.iter().any(|u| u == &normalized);
    let already_removed = store.removed_defaults.iter().any(|r| r == &normalized);
    let active_default = is_default && !already_removed;

    if !active_default && !in_user {
        return Err(ProtectedPathsError::NotFound(normalized));
    }

    if is_default && !already_removed {
        store.removed_defaults.push(normalized.clone());
    }
    store.user.retain(|u| u != &normalized);
    save_store(data_dir, &store)
}

/// Restore the implicit defaults — clears both deltas, removing the
/// store file entirely if writes succeed.
pub fn reset(data_dir: &Path) -> Result<(), ProtectedPathsError> {
    let path = store_path(data_dir);
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn list_returns_defaults_when_no_store() {
        let d = tmp_dir();
        let got = list(d.path()).unwrap();
        assert_eq!(got.len(), DEFAULT_PATHS.len());
        assert!(got.iter().all(|p| p.source == PathSource::Default));
        assert_eq!(got[0].path, "/");
    }

    #[test]
    fn add_user_path_persists() {
        let d = tmp_dir();
        let p = add(d.path(), "/Volumes/work").unwrap();
        assert_eq!(p.source, PathSource::User);
        assert_eq!(p.path, "/Volumes/work");
        let got = list(d.path()).unwrap();
        assert!(got.iter().any(|q| q.path == "/Volumes/work" && q.source == PathSource::User));
    }

    #[test]
    fn add_normalizes_trailing_slash() {
        let d = tmp_dir();
        add(d.path(), "/Volumes/work/").unwrap();
        let got = list(d.path()).unwrap();
        assert!(got.iter().any(|q| q.path == "/Volumes/work"));
    }

    #[test]
    fn add_root_stays_root() {
        let d = tmp_dir();
        // "/" is already a default; un-removing nothing means duplicate.
        let err = add(d.path(), "/").unwrap_err();
        assert!(matches!(err, ProtectedPathsError::Duplicate(_)));
    }

    #[test]
    fn add_rejects_relative_path() {
        let d = tmp_dir();
        let err = add(d.path(), "relative/path").unwrap_err();
        assert!(matches!(err, ProtectedPathsError::NotAbsolute(_)));
    }

    #[test]
    fn add_rejects_empty() {
        let d = tmp_dir();
        let err = add(d.path(), "   ").unwrap_err();
        assert!(matches!(err, ProtectedPathsError::Empty));
    }

    #[test]
    fn add_rejects_duplicate_user() {
        let d = tmp_dir();
        add(d.path(), "/x").unwrap();
        let err = add(d.path(), "/x").unwrap_err();
        assert!(matches!(err, ProtectedPathsError::Duplicate(_)));
    }

    #[test]
    fn add_accepts_tilde_and_tilde_path() {
        let d = tmp_dir();
        // "~" is a default → un-removing nothing means duplicate.
        assert!(matches!(
            add(d.path(), "~").unwrap_err(),
            ProtectedPathsError::Duplicate(_)
        ));
        // But "~/foo" is brand-new and accepted.
        let p = add(d.path(), "~/foo").unwrap();
        assert_eq!(p.path, "~/foo");
        assert_eq!(p.source, PathSource::User);
    }

    #[test]
    fn remove_default_tombstones_it() {
        let d = tmp_dir();
        remove(d.path(), "/tmp").unwrap();
        let got = list(d.path()).unwrap();
        assert!(!got.iter().any(|p| p.path == "/tmp"));
    }

    #[test]
    fn remove_user_drops_entry() {
        let d = tmp_dir();
        add(d.path(), "/Volumes/x").unwrap();
        remove(d.path(), "/Volumes/x").unwrap();
        let got = list(d.path()).unwrap();
        assert!(!got.iter().any(|p| p.path == "/Volumes/x"));
    }

    #[test]
    fn remove_unknown_errors() {
        let d = tmp_dir();
        let err = remove(d.path(), "/never/added").unwrap_err();
        assert!(matches!(err, ProtectedPathsError::NotFound(_)));
    }

    #[test]
    fn add_un_removes_default() {
        let d = tmp_dir();
        remove(d.path(), "/tmp").unwrap();
        assert!(!list(d.path()).unwrap().iter().any(|p| p.path == "/tmp"));

        let p = add(d.path(), "/tmp").unwrap();
        assert_eq!(p.source, PathSource::Default);
        assert!(list(d.path()).unwrap().iter().any(|q| q.path == "/tmp"));
    }

    #[test]
    fn reset_restores_defaults() {
        let d = tmp_dir();
        remove(d.path(), "/tmp").unwrap();
        add(d.path(), "/Volumes/x").unwrap();
        reset(d.path()).unwrap();
        let got = list(d.path()).unwrap();
        assert_eq!(got.len(), DEFAULT_PATHS.len());
        assert!(got.iter().all(|p| p.source == PathSource::Default));
    }

    #[test]
    fn resolved_set_expands_tilde() {
        let d = tmp_dir();
        let set = resolved_set(d.path()).unwrap();
        // "~" default contributes both itself and the expanded HOME.
        assert!(set.contains("~"));
        if let Some(home) = dirs::home_dir() {
            assert!(set.contains(home.to_string_lossy().as_ref()));
        }
    }

    #[test]
    fn resolved_set_includes_root_default() {
        let d = tmp_dir();
        let set = resolved_set(d.path()).unwrap();
        assert!(set.contains("/"));
    }

    #[test]
    fn store_survives_roundtrip() {
        let d = tmp_dir();
        add(d.path(), "/a").unwrap();
        remove(d.path(), "/tmp").unwrap();
        // Force a re-read by listing fresh.
        let got = list(d.path()).unwrap();
        assert!(got.iter().any(|p| p.path == "/a" && p.source == PathSource::User));
        assert!(!got.iter().any(|p| p.path == "/tmp"));
    }

    #[test]
    fn invalid_json_errors() {
        let d = tmp_dir();
        fs::write(store_path(d.path()), "not json").unwrap();
        let err = list(d.path()).unwrap_err();
        assert!(matches!(err, ProtectedPathsError::InvalidJson { .. }));
    }

    #[test]
    fn empty_file_treated_as_empty_store() {
        let d = tmp_dir();
        fs::write(store_path(d.path()), "").unwrap();
        let got = list(d.path()).unwrap();
        assert_eq!(got.len(), DEFAULT_PATHS.len());
    }
}
