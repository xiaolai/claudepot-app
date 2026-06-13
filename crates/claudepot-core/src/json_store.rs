//! Generic persistence helpers for Claudepot's small JSON files.
//!
//! Two shapes live here, both previously copy-pasted per module with
//! demonstrated drift (only `agent::events::store` had received the
//! grill-X23 corrupt-rename fix; the three older copies still
//! silently clobbered forensics):
//!
//! 1. **Schema-versioned JSON store** — [`load_or_recover`] /
//!    [`load`] / [`load_or_default`] / [`save`]. Used by
//!    `rotation::store`, `rotation::breaker_store`,
//!    `permission::store`, and `agent::events::store`. Each store
//!    keeps only its filename const, error enum, `Validate` impl,
//!    and domain tests; the corruption policy lives here so any
//!    future change lands in one diff.
//! 2. **Capped JSON ring-buffer log** — [`CappedJsonLog`]. The
//!    `Mutex<Inner>` + `VecDeque` + cap + write-through +
//!    poison-recovery + corrupt-rename engine shared by
//!    `notification_log` and `rotation::audit`.
//!
//! ## The three-outcome load contract
//!
//! - `Ok(..)` — successfully read + parsed + validated, OR the file
//!   didn't exist (default returned), OR the file existed but was
//!   corrupt/invalid (moved aside; default returned, with a
//!   [`CorruptionRecovery`] marker from [`load_or_recover`]).
//! - `Err(io_error)` — a *real* filesystem failure (permission
//!   denied, transient I/O error, disk unmounted). The caller must
//!   refuse to act on the assumption "empty store" until the error
//!   is resolved — silently treating a permission failure as empty
//!   and then saving would clobber the user's real file.
//!
//! Corruption recovery is intentionally NOT an error case: a missing
//! or unparseable file is a recoverable steady state, and the
//! rename-aside preserves forensics. Stores whose contents carry
//! revert obligations (permission grants) use [`load_or_recover`]
//! and surface the marker loudly instead of swallowing it.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::fs_utils::atomic_write;

// ───────────────────────── schema-versioned store ──────────────────

/// Domain validation hooked into [`load_or_recover`] and [`save`].
/// A file that parses but fails `validate` is treated exactly like a
/// corrupt file on load (moved aside, default returned) and is
/// refused on save (so on-disk files are always loadable).
pub trait Validate {
    /// Domain validation error. Only `Display` is required — the
    /// load path stringifies it into the warn log and the
    /// [`CorruptionRecovery`] marker.
    type Error: std::fmt::Display;
    fn validate(&self) -> Result<(), Self::Error>;
}

/// Marker that a load recovered from a corrupt or invalid file by
/// moving it aside and starting from the default value.
#[derive(Debug, Clone)]
pub struct CorruptionRecovery {
    /// Display form of the parse/validation error that triggered the
    /// recovery.
    pub error: String,
    /// Where the corrupt file was moved (`None` when the rename
    /// itself failed; the warn log carries the OS error).
    pub moved_to: Option<PathBuf>,
}

/// Result of [`load_or_recover`]: the usable value plus an optional
/// recovery marker. `recovery: Some(..)` means the on-disk file was
/// corrupt/invalid and `value` is the default — callers for whom
/// "silently empty" is dangerous (permission grants) inspect this
/// and surface it to the user.
#[derive(Debug)]
pub struct Loaded<T> {
    pub value: T,
    pub recovery: Option<CorruptionRecovery>,
}

/// Load a schema-versioned JSON file under the three-outcome
/// contract (see the module docs), reporting corruption recovery
/// explicitly instead of swallowing it.
pub fn load_or_recover<T>(path: &Path, store: &'static str) -> std::io::Result<Loaded<T>>
where
    T: DeserializeOwned + Default + Validate,
{
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Loaded {
                value: T::default(),
                recovery: None,
            });
        }
        Err(e) => return Err(e),
    };
    let error = match serde_json::from_slice::<T>(&bytes) {
        Ok(value) => match value.validate() {
            Ok(()) => {
                return Ok(Loaded {
                    value,
                    recovery: None,
                })
            }
            Err(e) => {
                tracing::warn!(
                    store,
                    error = %e,
                    "json_store: parsed but invalid; moving aside and starting empty"
                );
                e.to_string()
            }
        },
        Err(e) => {
            tracing::warn!(
                store,
                error = %e,
                "json_store: parse failed; moving aside and starting empty"
            );
            e.to_string()
        }
    };
    let moved_to = move_aside(path, store);
    Ok(Loaded {
        value: T::default(),
        recovery: Some(CorruptionRecovery { error, moved_to }),
    })
}

/// [`load_or_recover`] with the recovery marker discarded — the
/// historical `load_from` shape every store keeps for callers that
/// only need the three-outcome contract.
pub fn load<T>(path: &Path, store: &'static str) -> std::io::Result<T>
where
    T: DeserializeOwned + Default + Validate,
{
    Ok(load_or_recover(path, store)?.value)
}

/// Log + swallow real I/O errors, always returning a usable value.
/// Use only where errors cannot be propagated; new code prefers
/// [`load`].
pub fn load_or_default<T>(path: &Path, store: &'static str) -> T
where
    T: DeserializeOwned + Default + Validate,
{
    match load(path, store) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                store,
                error = %e,
                "json_store: read failed; defaulting to empty (real failures should \
                 propagate, but caller asked for default)"
            );
            T::default()
        }
    }
}

/// Save failure: validation rejected the value, serialization
/// failed, or the atomic write failed. Each store maps these onto
/// its own error enum so the public store APIs stay unchanged.
#[derive(Debug)]
pub enum SaveError<E> {
    Validation(E),
    Serde(serde_json::Error),
    Io(std::io::Error),
}

/// Persist `value` to `path`: validate → pretty-serialize →
/// [`atomic_write`] (temp + fsync + rename; 0600 on Unix). Invalid
/// input is rejected so on-disk files are always loadable.
pub fn save<T>(path: &Path, value: &T) -> Result<(), SaveError<T::Error>>
where
    T: Serialize + Validate,
{
    value.validate().map_err(SaveError::Validation)?;
    let json = serde_json::to_vec_pretty(value).map_err(SaveError::Serde)?;
    atomic_write(path, &json).map_err(SaveError::Io)?;
    Ok(())
}

/// Rename a corrupt file out of the way so the next load starts
/// empty. Returns the corrupt copy's path when the rename succeeded.
///
/// grill X23 (originally fixed only in `agent::events::store`, now
/// the shared policy): previously the corrupt filename was a fixed
/// `<path>.json.corrupt` and the rename's failure was silently
/// dropped — repeated corruption events would overwrite the forensic
/// copy, and a permission/EXDEV/disk-full rename failure looked
/// identical to a successful move-aside from the caller's side.
///
/// Two behaviors:
///
/// 1. The corrupt filename carries a unix-second suffix
///    (`<path>.corrupt.<unix-ts>`) so two corruption events seconds
///    apart land on different files and neither overwrites the
///    other. The seconds-resolution timestamp matches the rest of
///    Claudepot's filesystem breadcrumbs (run dirs,
///    `dispatch-failed-<ts>-<session>`) and keeps the filename
///    sortable.
/// 2. A rename failure is logged at `warn!` with the original path,
///    the corrupt target, and the OS error so a recurring corruption
///    (e.g. read-only home, parent dir missing) is visible. The
///    caller still recovers by returning the default — losing a
///    single forensic copy is preferable to refusing to load.
pub fn move_aside(path: &Path, store: &'static str) -> Option<PathBuf> {
    let suffix = chrono::Utc::now().timestamp();
    let filename = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "claudepot".to_string());
    let mut corrupt = path.to_path_buf();
    corrupt.set_file_name(format!("{filename}.corrupt.{suffix}"));
    match std::fs::rename(path, &corrupt) {
        Ok(()) => Some(corrupt),
        Err(e) => {
            tracing::warn!(
                store,
                from = %path.display(),
                to = %corrupt.display(),
                error = %e,
                "json_store: failed to move corrupt file aside; the next load \
                 will retry but the forensic copy was lost"
            );
            None
        }
    }
}

/// Every forensic corrupt copy sitting next to `path` — both the
/// X23 timestamped shape (`<name>.corrupt.<unix-ts>`) and the legacy
/// fixed shape (`<name>.corrupt`) written by pre-X23 builds. Sorted
/// for deterministic display.
///
/// Lets a caller (orchestrator / UI) detect that a store recovered
/// from corruption in an earlier process — e.g. surface "the grants
/// file was unreadable; elevated projects may not auto-revert" even
/// when the recovery happened before this boot.
pub fn corrupt_siblings(path: &Path) -> Vec<PathBuf> {
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let Some(base) = path.file_name().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    let prefix = format!("{base}.corrupt");
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|s| s.to_str()).is_some_and(|n| {
                let Some(rest) = n.strip_prefix(prefix.as_str()) else {
                    return false;
                };
                rest.is_empty()
                    || rest
                        .strip_prefix('.')
                        .is_some_and(|ts| !ts.is_empty() && ts.chars().all(|c| c.is_ascii_digit()))
            })
        })
        .collect();
    out.sort();
    out
}

// ───────────────────────── capped ring-buffer log ──────────────────

/// Entries in a [`CappedJsonLog`] carry a monotonic per-process id,
/// assigned by the engine on append (reset to max+1 on load).
pub trait HasId {
    fn id(&self) -> u64;
    fn set_id(&mut self, id: u64);
}

/// Default meta for logs that persist nothing beyond their entries
/// (`rotation::audit`). Logs with extra persisted state declare
/// their own meta struct (`notification_log`'s read cursor) — it is
/// `#[serde(flatten)]`ed into the top level of the on-disk doc, so
/// the historical file shapes round-trip unchanged.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct NoMeta {}

/// Static configuration for one log: the `name` lands in log
/// messages, poison-recovery warns, and the volatile temp path;
/// `cap` is the ring-buffer bound; `pretty` selects pretty vs
/// compact JSON on disk (the two existing logs differ).
#[derive(Debug, Clone, Copy)]
pub struct LogConfig {
    pub name: &'static str,
    pub cap: usize,
    pub pretty: bool,
}

/// Under-lock state of a [`CappedJsonLog`]. Domain modules reach the
/// entries/meta through [`CappedJsonLog::with`] /
/// [`CappedJsonLog::with_mut`] to implement their own queries
/// (filtered lists, unread counts, delivery marks) without
/// re-implementing the persistence engine.
pub struct LogState<E, M> {
    path: PathBuf,
    pub(crate) entries: VecDeque<E>,
    pub(crate) next_id: u64,
    pub(crate) meta: M,
    /// When true, persistence is skipped entirely. Set by
    /// [`CappedJsonLog::in_memory_only`] for the boot-fallback path
    /// so a degraded log doesn't spam attempted writes against a
    /// path that's already known to be unreachable.
    volatile: bool,
}

/// Serialized doc shape. `meta` is flattened so a log with
/// `NoMeta` writes `{"entries": [...]}` and `notification_log`
/// writes `{"last_seen_id": N, "entries": [...]}` — byte-compatible
/// with the shapes both modules wrote before the extraction.
#[derive(Serialize)]
struct DocRef<'a, E: Serialize, M: Serialize> {
    #[serde(flatten)]
    meta: &'a M,
    entries: &'a VecDeque<E>,
}

#[derive(Deserialize)]
struct DocOwned<E, M> {
    #[serde(flatten)]
    meta: M,
    #[serde(default = "VecDeque::new")]
    entries: VecDeque<E>,
}

/// Shared engine for capped, write-through JSON ring-buffer logs:
/// single mutex (poison-recovered via [`crate::sync::recover_lock`]),
/// `VecDeque` with O(1) front eviction, monotonic id assignment,
/// full-doc atomic write-through, corrupt-file rename-aside on open.
/// Append rate on every current log is user-paced, so the lock is
/// never contended and write-through beats a debounce path that
/// could lose entries on crash.
pub struct CappedJsonLog<E, M = NoMeta> {
    cfg: LogConfig,
    inner: Mutex<LogState<E, M>>,
}

impl<E, M> CappedJsonLog<E, M>
where
    E: HasId + Serialize + DeserializeOwned,
    M: Serialize + DeserializeOwned + Default,
{
    /// Build a volatile in-memory-only log — appends and reads work,
    /// but persistence is skipped entirely. Used as the last-resort
    /// boot fallback when the on-disk path refused to open, and by
    /// tests that don't want filesystem dependencies. Nothing
    /// survives a restart.
    ///
    /// The `path` field is kept (set to a unique per-process value)
    /// so the state shape is uniform across both code paths, but
    /// nothing ever writes to it.
    pub fn in_memory_only(cfg: LogConfig) -> Self {
        let path = std::env::temp_dir().join(format!(
            "claudepot-{}-volatile-{}",
            cfg.name,
            std::process::id()
        ));
        Self {
            cfg,
            inner: Mutex::new(LogState {
                path,
                entries: VecDeque::new(),
                next_id: 1,
                meta: M::default(),
                volatile: true,
            }),
        }
    }

    /// Open the log at `path`. A missing file is an empty log; a
    /// corrupt file is moved aside ([`move_aside`], X23-grade) and
    /// the log starts empty. Both cases are non-fatal — better than
    /// wedging the surface on a parse glitch. A *real* I/O failure
    /// propagates as `Err`.
    pub fn open(path: PathBuf, cfg: LogConfig) -> std::io::Result<Self> {
        let empty = || DocOwned::<E, M> {
            meta: M::default(),
            entries: VecDeque::new(),
        };
        let doc = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<DocOwned<E, M>>(&bytes) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(
                        store = cfg.name,
                        error = %e,
                        "json_store: log parse failed; moving aside and starting empty"
                    );
                    move_aside(&path, cfg.name);
                    empty()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => empty(),
            Err(e) => return Err(e),
        };
        // Defense against a hand-edited file that exceeds the cap —
        // truncate from the front so we keep the newest tail.
        let mut entries = doc.entries;
        while entries.len() > cfg.cap {
            entries.pop_front();
        }
        let next_id = entries.iter().map(|e| e.id()).max().unwrap_or(0) + 1;
        Ok(Self {
            cfg,
            inner: Mutex::new(LogState {
                path,
                entries,
                next_id,
                meta: doc.meta,
                volatile: false,
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LogState<E, M>> {
        crate::sync::recover_lock(&self.inner, self.cfg.name)
    }

    /// Append `entry`, assigning the next monotonic id (the caller's
    /// id field is overwritten), evicting from the front past the
    /// cap, and persisting write-through. Returns the assigned id.
    pub fn append(&self, mut entry: E) -> std::io::Result<u64> {
        let mut g = self.lock();
        let id = g.next_id;
        g.next_id = g.next_id.saturating_add(1);
        entry.set_id(id);
        g.entries.push_back(entry);
        while g.entries.len() > self.cfg.cap {
            g.entries.pop_front();
        }
        self.persist(&g)?;
        Ok(id)
    }

    /// Run a read-only closure under the lock. Domain queries
    /// (filtered lists, unread counts) live in the owning module.
    pub fn with<R>(&self, f: impl FnOnce(&LogState<E, M>) -> R) -> R {
        f(&self.lock())
    }

    /// Run a mutating closure under the lock. The closure returns
    /// `(result, persist)`; when `persist` is true the full doc is
    /// written through before the lock is released, so the on-disk
    /// file always matches the in-memory state at the release point.
    /// Returning `false` skips the write (idempotent no-ops,
    /// in-memory normalization after open).
    pub fn with_mut<R>(
        &self,
        f: impl FnOnce(&mut LogState<E, M>) -> (R, bool),
    ) -> std::io::Result<R> {
        let mut g = self.lock();
        let (r, persist) = f(&mut g);
        if persist {
            self.persist(&g)?;
        }
        Ok(r)
    }

    /// Total entry count.
    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Persist the current state to disk. Caller must hold the inner
    /// lock. No-op when the log is volatile.
    fn persist(&self, g: &LogState<E, M>) -> std::io::Result<()> {
        if g.volatile {
            return Ok(());
        }
        let doc = DocRef {
            meta: &g.meta,
            entries: &g.entries,
        };
        let bytes = if self.cfg.pretty {
            serde_json::to_vec_pretty(&doc)
        } else {
            serde_json::to_vec(&doc)
        }
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{} serialize: {e}", self.cfg.name),
            )
        })?;
        atomic_write(&g.path, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── schema-versioned store ──────────────────────────────────────

    const TEST_SCHEMA_VERSION: u32 = 1;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TestFile {
        schema_version: u32,
        items: Vec<String>,
    }

    impl Default for TestFile {
        fn default() -> Self {
            Self {
                schema_version: TEST_SCHEMA_VERSION,
                items: Vec::new(),
            }
        }
    }

    impl Validate for TestFile {
        type Error = String;
        fn validate(&self) -> Result<(), String> {
            if self.schema_version != TEST_SCHEMA_VERSION {
                return Err(format!(
                    "schema version {} is unsupported",
                    self.schema_version
                ));
            }
            if self.items.iter().any(|i| i.is_empty()) {
                return Err("empty item".to_string());
            }
            Ok(())
        }
    }

    #[test]
    fn test_json_store_load_missing_file_yields_default() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("nope.json");
        let f: TestFile = load(&p, "test").unwrap();
        assert_eq!(f, TestFile::default());
        let l: Loaded<TestFile> = load_or_recover(&p, "test").unwrap();
        assert!(l.recovery.is_none(), "missing file is not a recovery");
    }

    #[test]
    fn test_json_store_save_then_load_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("store.json");
        let f = TestFile {
            schema_version: TEST_SCHEMA_VERSION,
            items: vec!["a".into(), "b".into()],
        };
        save(&p, &f).unwrap();
        let back: Loaded<TestFile> = load_or_recover(&p, "test").unwrap();
        assert_eq!(back.value, f);
        assert!(back.recovery.is_none());
    }

    #[test]
    fn test_json_store_save_rejects_invalid_value() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("store.json");
        let bad = TestFile {
            schema_version: 99,
            items: Vec::new(),
        };
        let err = save(&p, &bad);
        assert!(matches!(err, Err(SaveError::Validation(_))));
        assert!(!p.exists(), "rejected file must never be written");
    }

    #[test]
    fn test_json_store_corrupt_file_is_moved_aside_with_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("store.json");
        std::fs::write(&p, b"this is not json").unwrap();
        let l: Loaded<TestFile> = load_or_recover(&p, "test").unwrap();
        assert_eq!(l.value, TestFile::default());
        let rec = l.recovery.expect("corruption must be reported");
        let copies = corrupt_siblings(&p);
        assert_eq!(
            copies.len(),
            1,
            "corrupt file should be moved aside under a timestamped name"
        );
        assert_eq!(rec.moved_to.as_deref(), Some(copies[0].as_path()));
        assert!(!p.exists(), "original file must be gone after move-aside");
    }

    #[test]
    fn test_json_store_invalid_but_parsable_file_is_moved_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("store.json");
        // Parses fine, fails validate (bad schema version).
        std::fs::write(&p, br#"{"schema_version":99,"items":[]}"#).unwrap();
        let l: Loaded<TestFile> = load_or_recover(&p, "test").unwrap();
        assert_eq!(l.value, TestFile::default());
        let rec = l.recovery.expect("validation failure must be reported");
        assert!(
            rec.error.contains("unsupported"),
            "marker carries the validation error: {}",
            rec.error
        );
        assert_eq!(corrupt_siblings(&p).len(), 1);
    }

    #[test]
    fn test_json_store_repeated_corruption_does_not_overwrite_forensic_copy() {
        // grill X23: previously every corrupt-load wrote to a fixed
        // `<path>.json.corrupt` and the second corruption clobbered
        // the first forensic copy. With the timestamp suffix, each
        // corruption that surfaces in a different second lands on a
        // different file and accumulates.
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("store.json");

        std::fs::write(&p, b"first corruption").unwrap();
        let _: TestFile = load(&p, "test").unwrap();
        let copies1 = corrupt_siblings(&p);
        assert_eq!(copies1.len(), 1);

        // Sleep just past the 1-second timestamp resolution before
        // the second corruption — otherwise the second rename would
        // try to land on the same name. The assertion below is
        // intentionally permissive in the same-second case: the bug
        // fixed is "every corruption silently overwrites the prior
        // copy", not "every corruption N milliseconds apart gets its
        // own copy".
        std::thread::sleep(std::time::Duration::from_secs(1));
        std::fs::write(&p, b"second corruption").unwrap();
        let _: TestFile = load(&p, "test").unwrap();
        let copies2 = corrupt_siblings(&p);
        assert!(
            !copies2.is_empty(),
            "at least one forensic copy must always exist"
        );
        assert!(
            copies2.iter().any(|c| c == &copies1[0]),
            "the first forensic copy is not clobbered by a later corruption"
        );
    }

    #[test]
    fn test_json_store_corrupt_siblings_matches_legacy_fixed_name() {
        // Pre-X23 builds wrote `<name>.corrupt` (fixed). Detection
        // must still see those so the UI can surface recoveries that
        // happened before an upgrade.
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("store.json");
        std::fs::write(tmp.path().join("store.json.corrupt"), b"old").unwrap();
        std::fs::write(tmp.path().join("store.json.corrupt.1700000000"), b"new").unwrap();
        // Near-misses must not match.
        std::fs::write(tmp.path().join("store.json.corrupted"), b"x").unwrap();
        std::fs::write(tmp.path().join("store.json.corrupt.abc"), b"x").unwrap();
        let copies = corrupt_siblings(&p);
        assert_eq!(copies.len(), 2, "legacy + timestamped match: {copies:?}");
    }

    #[cfg(unix)]
    #[test]
    fn test_json_store_permission_denied_returns_err_not_default() {
        // Regression for the silent-clobber bug: a permission error
        // on read used to look like "empty store" — a follow-up save
        // would then write a fresh file and lose the user's real
        // config. The error must propagate so the caller can refuse
        // to act on the assumption.
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("store.json");
        std::fs::write(&p, br#"{"schema_version":1,"items":[]}"#).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).unwrap();
        let result: std::io::Result<TestFile> = load(&p, "test");
        // Restore permissions so the tempdir cleanup can delete it.
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(result.is_err(), "permission denied must surface as Err");
    }

    #[cfg(unix)]
    #[test]
    fn test_json_store_save_writes_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("store.json");
        save(&p, &TestFile::default()).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn test_json_store_load_or_default_swallows_parse_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("store.json");
        std::fs::write(&p, b"garbage").unwrap();
        let f: TestFile = load_or_default(&p, "test");
        assert_eq!(f, TestFile::default());
    }

    // ── capped ring-buffer log ──────────────────────────────────────

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TestEntry {
        id: u64,
        label: String,
    }

    impl HasId for TestEntry {
        fn id(&self) -> u64 {
            self.id
        }
        fn set_id(&mut self, id: u64) {
            self.id = id;
        }
    }

    #[derive(Debug, Default, Serialize, Deserialize)]
    struct TestMeta {
        #[serde(default)]
        cursor: u64,
    }

    const CFG: LogConfig = LogConfig {
        name: "test_log",
        cap: 5,
        pretty: false,
    };

    fn entry(label: &str) -> TestEntry {
        TestEntry {
            id: 0,
            label: label.to_string(),
        }
    }

    #[test]
    fn test_capped_log_append_assigns_monotonic_ids_and_caps() {
        let log: CappedJsonLog<TestEntry> = CappedJsonLog::in_memory_only(CFG);
        let mut last = 0;
        for i in 0..(CFG.cap + 3) {
            let id = log.append(entry(&format!("e{i}"))).unwrap();
            assert!(id > last, "ids must be monotonic");
            last = id;
        }
        assert_eq!(log.len(), CFG.cap, "ring caps at cfg.cap");
        // Oldest entries evicted from the front.
        let head = log.with(|s| s.entries.front().cloned().unwrap());
        assert_eq!(head.label, "e3");
    }

    #[test]
    fn test_capped_log_persistence_roundtrip_with_flattened_meta() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("log.json");
        {
            let log: CappedJsonLog<TestEntry, TestMeta> =
                CappedJsonLog::open(path.clone(), CFG).unwrap();
            log.append(entry("alpha")).unwrap();
            log.append(entry("beta")).unwrap();
            log.with_mut(|s| {
                s.meta.cursor = 7;
                ((), true)
            })
            .unwrap();
        }
        // The meta must flatten to a TOP-LEVEL key, matching the
        // historical doc shapes (`{"last_seen_id": N, "entries": …}`).
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["cursor"], 7);
        assert!(raw["entries"].is_array());

        let log: CappedJsonLog<TestEntry, TestMeta> = CappedJsonLog::open(path, CFG).unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log.with(|s| s.meta.cursor), 7);
        // ID continuity — the next append must not collide.
        let id = log.append(entry("gamma")).unwrap();
        assert_eq!(id, 3);
    }

    #[test]
    fn test_capped_log_nometa_doc_shape_is_entries_only() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("log.json");
        let log: CappedJsonLog<TestEntry> = CappedJsonLog::open(path.clone(), CFG).unwrap();
        log.append(entry("x")).unwrap();
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let obj = raw.as_object().unwrap();
        assert_eq!(obj.len(), 1, "NoMeta adds no keys: {obj:?}");
        assert!(obj.contains_key("entries"));
    }

    #[test]
    fn test_capped_log_corrupt_file_moved_aside_on_open() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("log.json");
        std::fs::write(&path, b"{not valid json").unwrap();
        let log: CappedJsonLog<TestEntry> = CappedJsonLog::open(path.clone(), CFG).unwrap();
        assert!(log.is_empty());
        assert_eq!(corrupt_siblings(&path).len(), 1);
    }

    #[test]
    fn test_capped_log_open_truncates_over_cap_from_front() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("log.json");
        let entries: Vec<TestEntry> = (1..=8)
            .map(|i| TestEntry {
                id: i,
                label: format!("e{i}"),
            })
            .collect();
        let doc = serde_json::json!({ "entries": entries });
        std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        let log: CappedJsonLog<TestEntry> = CappedJsonLog::open(path, CFG).unwrap();
        assert_eq!(log.len(), CFG.cap);
        assert_eq!(log.with(|s| s.entries.front().unwrap().id), 4);
        // next_id continues past the persisted max.
        assert_eq!(log.append(entry("new")).unwrap(), 9);
    }

    #[test]
    fn test_capped_log_volatile_never_touches_disk() {
        let log: CappedJsonLog<TestEntry> = CappedJsonLog::in_memory_only(CFG);
        log.append(entry("x")).unwrap();
        let path = log.with(|s| s.path.clone());
        assert!(!path.exists(), "volatile log must not write its path");
    }

    #[test]
    fn test_capped_log_with_mut_skips_persist_when_false() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("log.json");
        let log: CappedJsonLog<TestEntry, TestMeta> =
            CappedJsonLog::open(path.clone(), CFG).unwrap();
        log.with_mut(|s| {
            s.meta.cursor = 42;
            ((), false)
        })
        .unwrap();
        assert!(
            !path.exists(),
            "persist=false must not create the file on disk"
        );
    }

    #[test]
    fn test_capped_log_survives_poisoned_mutex() {
        use std::sync::Arc;
        let log: Arc<CappedJsonLog<TestEntry>> = Arc::new(CappedJsonLog::in_memory_only(CFG));
        log.append(entry("before")).unwrap();
        let l2 = Arc::clone(&log);
        let join = std::thread::spawn(move || {
            l2.with(|_| panic!("intentional panic to poison the mutex"))
        });
        let _ = join.join();
        // recover_lock keeps the log usable after the poisoning.
        let id = log.append(entry("after")).unwrap();
        assert_eq!(id, 2);
        assert_eq!(log.len(), 2);
    }
}
