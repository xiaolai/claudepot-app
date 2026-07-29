//! The one serialized read-modify-write boundary for CC's JSON settings
//! files.
//!
//! # Why atomic writes are not enough
//!
//! [`crate::fs_utils::atomic_write`] gives **crash-safety**: a reader sees
//! either the whole old file or the whole new one, never a torn write. It does
//! not give **concurrency-safety**. Two overlapping read-modify-writes both
//! read the old bytes, each applies its own edit to that stale object, and the
//! later rename silently discards the earlier mutation. The file is intact and
//! one of the two edits is simply gone.
//!
//! That was a rare race while every writer was a one-shot user action. A pane
//! with per-row optimistic writes makes overlap the normal case, so every
//! same-process writer of these files now goes through
//! [`mutate_settings_file`]: `settings_writer` (which covers retention,
//! models, attribution, fast mode, artifact and memory in one change, since
//! they all route through its helpers), `updates::settings_bridge`,
//! `permission::settings`, and `cc_env`.
//!
//! A lock only one participant holds is not a lock, which is why the migration
//! is not deferred: `cc_env` writing `~/.claude/settings.json` under a mutex
//! that `settings_bridge` ignores would be exactly as racy as no mutex at all.
//!
//! # What this does and does not guarantee
//!
//! **Same-process writes are serialized.** A per-path mutex means two
//! Claudepot threads never interleave a read-modify-write of one file.
//!
//! **External writes cannot be.** Claude Code itself and a user's text editor
//! do not honor Claudepot's mutex. They get *change-detection*: the file is
//! re-read immediately before the write, and if the bytes moved since we
//! parsed them the closure is re-run against the newer object rather than
//! overwriting a change we never saw. That closes the window to the
//! microseconds between the final read and the rename — it does not close it
//! entirely, and no in-process construct can. Claiming mutual exclusion
//! against an external process would be the kind of guarantee that reads as
//! safety and isn't.
//!
//! For the same reason [`Mutation::state`] is the state we just wrote rather
//! than a re-read: an external writer can land immediately after either one,
//! so a re-read would buy nothing but a second chance to be stale.

use crate::fs_utils::atomic_write;
use serde_json::{Map, Value as JsonValue};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// How many times a mutation is re-run when an external writer moves the file
/// underneath it. Small on purpose: a file changing four times inside one
/// mutation is a runaway writer, and looping forever would hang the caller.
const MAX_ATTEMPTS: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum SettingsMutexError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json parse: {0}")]
    JsonParse(#[from] serde_json::Error),
    #[error("settings file is not a JSON object at {0}")]
    NotAJsonObject(PathBuf),
    #[error("{path} kept changing underneath us across {attempts} attempts")]
    Contended { path: PathBuf, attempts: usize },
}

/// Whether the file existed when this attempt read it.
///
/// Passed to the closure because "created only when actually writing" is a
/// per-caller rule: `mutate_settings` must not litter an empty `{}` when a
/// removal-only edit runs against a missing file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileWas {
    Present,
    Absent,
}

impl FileWas {
    pub fn is_present(self) -> bool {
        self == Self::Present
    }
}

/// What the closure decided.
#[derive(Debug)]
pub enum Change<T> {
    /// Persist the mutated object.
    Write(T),
    /// Nothing to persist. The file is left byte-for-byte as it was, even if
    /// the closure mutated its copy of the object before changing its mind.
    Skip(T),
}

/// The result of one mutation.
#[derive(Debug)]
pub struct Mutation<T> {
    /// Whatever the closure returned.
    pub value: T,
    /// The authoritative post-write state: exactly the object now on disk
    /// (or, for a skip, exactly the object that was already there). Callers
    /// hand this back to the renderer so it reconciles against truth rather
    /// than against its own optimism.
    pub state: Map<String, JsonValue>,
    /// Whether anything was written.
    pub wrote: bool,
    /// Whether the closure had to be re-run because the file moved under it.
    pub rebased: bool,
}

fn locks() -> &'static Mutex<HashMap<PathBuf, Arc<Mutex<()>>>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Key a lock by the canonical parent plus the file name.
///
/// The file itself frequently does not exist yet, so it cannot be
/// canonicalized. Neither, sometimes, can its parent —
/// `<project>/.claude/settings.local.json` is written before `.claude/`
/// exists. So the key is built from the **nearest existing ancestor** plus
/// whatever components sit below it, which keeps `./x/settings.json` and
/// `/cwd/x/settings.json` on one lock even when `x/` does not exist yet.
/// Falling back to the raw path there, as an earlier version did, gave those
/// two callers different locks and let the later write discard the earlier
/// one — the exact failure this module exists to prevent.
///
/// Nothing is created here. A lock key must not have filesystem side effects:
/// `create_dir_all` belongs on the write path, after the closure has decided
/// there is anything to write.
///
/// `canonicalize` returns the `\\?\C:\…` verbatim form on Windows, so the
/// result goes through [`crate::path_utils::simplify_windows_path`] per
/// `rules/paths.md`; otherwise a caller that canonicalized and one that did
/// not would key two different locks onto the same file.
fn lock_key(path: &Path) -> PathBuf {
    // Anchor a relative path so it cannot key differently from its absolute
    // twin. `current_dir` failing is not a reason to give up — an unanchored
    // key is still better than none.
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|d| d.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };

    // Walk up to the nearest ancestor that exists, remembering what we
    // stepped over, then re-attach it to the canonical form.
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = absolute.as_path();
    loop {
        if let Ok(real) = std::fs::canonicalize(cursor) {
            return rejoin_tail(&normalize_anchor(&real), &tail);
        }
        match (cursor.file_name(), cursor.parent()) {
            (Some(name), Some(parent)) => {
                tail.push(name.to_os_string());
                cursor = parent;
            }
            // Reached the root without finding anything that exists.
            _ => return absolute,
        }
    }
}

/// Strip Windows' verbatim prefix from a canonicalized anchor.
///
/// Split out from [`lock_key`] because it is the only part of the key
/// computation that is pure — everything else touches the filesystem, and so
/// behaves differently on every host. `rules/paths.md` asks for golden tests
/// across all four path shapes on **all** host OSs; that is only possible
/// against a function that does no I/O.
fn normalize_anchor(canonical: &Path) -> PathBuf {
    match canonical.to_str() {
        Some(s) => PathBuf::from(crate::path_utils::simplify_windows_path(s)),
        // Non-UTF-8 paths keep their bytes rather than being lossily
        // rewritten — a lossy key could collide two different files.
        None => canonical.to_path_buf(),
    }
}

/// Re-attach the components walked over, outermost first.
///
/// `tail` is built inner-to-outer by the walk, so it is consumed in reverse.
/// Also pure, also golden-tested.
fn rejoin_tail(anchor: &Path, tail: &[std::ffi::OsString]) -> PathBuf {
    let mut key = anchor.to_path_buf();
    for part in tail.iter().rev() {
        key.push(part);
    }
    key
}

fn lock_for(path: &Path) -> Arc<Mutex<()>> {
    let key = lock_key(path);
    let mut map = locks().lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(map.entry(key).or_default())
}

/// The file's bytes, or `None` when it does not exist.
///
/// The one place "missing" is turned into a value rather than an error. Both
/// the parse path and the change-detection path need exactly this, and they
/// used to carry separate copies of it — in the two functions that decide
/// whether a concurrent write happened, which is the last place a divergence
/// would be noticed.
fn read_optional_bytes(path: &Path) -> Result<Option<Vec<u8>>, SettingsMutexError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Read the file as a JSON object. Missing or empty → an empty object.
/// Malformed or non-object → `Err`, never a silent overwrite.
fn read_object(
    path: &Path,
) -> Result<(Option<Vec<u8>>, Map<String, JsonValue>), SettingsMutexError> {
    let Some(bytes) = read_optional_bytes(path)? else {
        return Ok((None, Map::new()));
    };
    if bytes.is_empty() {
        return Ok((Some(bytes), Map::new()));
    }
    match serde_json::from_slice::<JsonValue>(&bytes)? {
        JsonValue::Object(map) => Ok((Some(bytes), map)),
        _ => Err(SettingsMutexError::NotAJsonObject(path.to_path_buf())),
    }
}

/// Serialize the pretty-printed, newline-terminated form every caller here
/// already produced independently.
///
/// "Format-preserving" would be the wrong word for any of this: JSON has no
/// comments and every existing writer in this crate parses and pretty-prints.
/// The guarantee is **key-preserving** — nothing the closure did not touch is
/// dropped, inside or outside the keys it manages.
fn serialize(object: &Map<String, JsonValue>) -> Result<Vec<u8>, SettingsMutexError> {
    let body = serde_json::to_string_pretty(&JsonValue::Object(object.clone()))?;
    let mut bytes = body.into_bytes();
    bytes.push(b'\n');
    Ok(bytes)
}

/// Run `mutate` against `path` under the per-path lock, with a rebase retry
/// when an external writer moves the file.
///
/// The closure may be called more than once and must therefore be free of
/// side effects outside the object it is handed.
pub fn mutate_settings_file<T, E>(
    path: &Path,
    mut mutate: impl FnMut(&mut Map<String, JsonValue>, FileWas) -> Result<Change<T>, E>,
) -> Result<Mutation<T>, E>
where
    E: From<SettingsMutexError>,
{
    let lock = lock_for(path);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());

    let mut rebased = false;
    for _ in 0..MAX_ATTEMPTS {
        let (before, original) = read_object(path)?;
        let was = if before.is_some() {
            FileWas::Present
        } else {
            FileWas::Absent
        };
        let mut object = original.clone();

        match mutate(&mut object, was)? {
            Change::Skip(value) => {
                // A skip still reports `state` as what is on disk, so it owes
                // the same change-detection a write does. Without it an
                // external writer landing during the closure would leave the
                // caller reconciling against bytes that no longer exist —
                // stale state presented as the authoritative one.
                if read_optional_bytes(path)? != before {
                    rebased = true;
                    continue;
                }
                return Ok(Mutation {
                    value,
                    state: original,
                    wrote: false,
                    rebased,
                });
            }
            Change::Write(value) => {
                // Change-detection for writers outside this process. Inside
                // it, the lock already made this impossible.
                if read_optional_bytes(path)? != before {
                    rebased = true;
                    continue;
                }
                let bytes = serialize(&object)?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(SettingsMutexError::Io)?;
                }
                atomic_write(path, &bytes).map_err(SettingsMutexError::Io)?;
                return Ok(Mutation {
                    value,
                    state: object,
                    wrote: true,
                    rebased,
                });
            }
        }
    }
    Err(SettingsMutexError::Contended {
        path: path.to_path_buf(),
        attempts: MAX_ATTEMPTS,
    }
    .into())
}

/// Read a settings file as a JSON object, taking the path's lock for the
/// duration of the read.
///
/// Reads are already tear-free thanks to atomic rename, so the lock buys
/// only one thing: this read cannot land in the middle of a same-process
/// mutation's read-modify-write. It does **not** extend over whatever the
/// caller decides next — the guard drops when this function returns, so a
/// caller that reads here and writes afterwards is doing its own
/// read-modify-write and should use [`mutate_settings_file`] instead.
pub fn read_settings_file(path: &Path) -> Result<Map<String, JsonValue>, SettingsMutexError> {
    let lock = lock_for(path);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    read_object(path).map(|(_, object)| object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Boundary(#[from] SettingsMutexError),
        #[error("closure said no")]
        Refused,
    }

    fn set(path: &Path, key: &str, value: JsonValue) -> Result<Mutation<()>, TestError> {
        mutate_settings_file(path, |o, _| {
            o.insert(key.to_string(), value.clone());
            Ok(Change::Write(()))
        })
    }

    #[test]
    fn writes_and_preserves_unrelated_keys() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("settings.json");
        std::fs::write(&p, br#"{"keep":{"nested":1}}"#).unwrap();

        let m = set(&p, "added", json!("x")).unwrap();
        assert!(m.wrote);
        assert!(!m.rebased);
        assert_eq!(m.state["added"], json!("x"));

        let after: JsonValue = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(after["keep"]["nested"], json!(1));
        assert_eq!(after["added"], json!("x"));
    }

    #[test]
    fn skip_leaves_the_file_byte_identical_even_after_the_closure_mutated_it() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("settings.json");
        let original = br#"{"a":1}"#.to_vec();
        std::fs::write(&p, &original).unwrap();

        let m: Mutation<()> = mutate_settings_file(&p, |o, _| {
            o.insert("scribble".into(), json!(true));
            Ok::<_, TestError>(Change::Skip(()))
        })
        .unwrap();

        assert!(!m.wrote);
        assert!(!m.state.contains_key("scribble"));
        assert_eq!(std::fs::read(&p).unwrap(), original);
    }

    #[test]
    fn malformed_file_errors_and_is_left_untouched() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("settings.json");
        std::fs::write(&p, b"{ not json").unwrap();

        let err = set(&p, "a", json!(1)).unwrap_err();
        assert!(matches!(
            err,
            TestError::Boundary(SettingsMutexError::JsonParse(_))
        ));
        assert_eq!(std::fs::read(&p).unwrap(), b"{ not json");
    }

    #[test]
    fn non_object_root_errors() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("settings.json");
        std::fs::write(&p, b"[1,2,3]").unwrap();
        let err = set(&p, "a", json!(1)).unwrap_err();
        assert!(matches!(
            err,
            TestError::Boundary(SettingsMutexError::NotAJsonObject(_))
        ));
    }

    #[test]
    fn closure_sees_whether_the_file_existed() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("settings.json");

        let m: Mutation<FileWas> =
            mutate_settings_file(&p, |_, was| Ok::<_, TestError>(Change::Skip(was))).unwrap();
        assert_eq!(m.value, FileWas::Absent);

        set(&p, "a", json!(1)).unwrap();
        let m: Mutation<FileWas> =
            mutate_settings_file(&p, |_, was| Ok::<_, TestError>(Change::Skip(was))).unwrap();
        assert_eq!(m.value, FileWas::Present);
    }

    #[test]
    fn closure_error_propagates_without_writing() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("settings.json");
        let r: Result<Mutation<()>, TestError> =
            mutate_settings_file(&p, |_, _| Err(TestError::Refused));
        assert!(matches!(r.unwrap_err(), TestError::Refused));
        assert!(!p.exists());
    }

    /// The whole point of the boundary: two writers, two different keys, both
    /// land. Under plain read-modify-write one of them is silently discarded.
    #[test]
    fn two_concurrent_writers_both_land() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("settings.json");
        std::fs::write(&p, b"{}").unwrap();

        let mut handles = Vec::new();
        for t in 0..8 {
            let path = p.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..25 {
                    let key = format!("t{t}_{i}");
                    mutate_settings_file(&path, |o, _| {
                        o.insert(key.clone(), json!(i));
                        Ok::<_, TestError>(Change::Write(()))
                    })
                    .unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let after: JsonValue = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        let object = after.as_object().unwrap();
        assert_eq!(object.len(), 8 * 25, "a write was lost");
        for t in 0..8 {
            for i in 0..25 {
                assert_eq!(object[&format!("t{t}_{i}")], json!(i));
            }
        }
    }

    /// An external writer (one that does not hold our lock) must not be
    /// overwritten: the closure re-runs against the newer bytes.
    #[test]
    fn external_change_between_read_and_write_forces_a_rebase() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("settings.json");
        std::fs::write(&p, br#"{"original":1}"#).unwrap();

        let calls = AtomicUsize::new(0);
        let path = p.clone();
        let m: Mutation<()> = mutate_settings_file(&p, move |o, _| {
            // First pass only: simulate Claude Code writing the file while we
            // were thinking. The second pass sees the newer object.
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                std::fs::write(&path, br#"{"original":1,"from_cc":true}"#).unwrap();
            }
            o.insert("ours".into(), json!("kept"));
            Ok::<_, TestError>(Change::Write(()))
        })
        .unwrap();

        assert!(m.rebased, "should have re-run against the newer file");
        let after: JsonValue = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(
            after["from_cc"],
            json!(true),
            "external write was clobbered"
        );
        assert_eq!(after["ours"], json!("kept"));
        assert_eq!(after["original"], json!(1));
    }

    #[test]
    fn a_file_that_never_settles_is_reported_not_looped_on() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("settings.json");
        std::fs::write(&p, b"{}").unwrap();

        let n = AtomicUsize::new(0);
        let path = p.clone();
        let r: Result<Mutation<()>, TestError> = mutate_settings_file(&p, move |o, _| {
            let i = n.fetch_add(1, Ordering::SeqCst);
            std::fs::write(&path, format!(r#"{{"churn":{i}}}"#)).unwrap();
            o.insert("ours".into(), json!(1));
            Ok(Change::Write(()))
        });
        match r.unwrap_err() {
            TestError::Boundary(SettingsMutexError::Contended { attempts, .. }) => {
                assert_eq!(attempts, MAX_ATTEMPTS)
            }
            other => panic!("expected Contended, got {other:?}"),
        }
    }

    #[test]
    fn relative_and_absolute_paths_share_one_lock() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("settings.json");
        std::fs::write(&p, b"{}").unwrap();
        let messy = tmp.path().join(".").join("settings.json");
        assert_eq!(lock_key(&p), lock_key(&messy));
    }

    /// `<project>/.claude/settings.local.json` is written before `.claude/`
    /// exists. Two spellings of that path must still key one lock — falling
    /// back to the raw path here gave them different locks, and the later
    /// write then discarded the earlier one.
    #[test]
    fn paths_below_a_missing_directory_still_share_one_lock() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("dot-claude").join("settings.local.json");
        assert!(!missing.parent().unwrap().exists());
        let messy = tmp
            .path()
            .join(".")
            .join("dot-claude")
            .join("settings.local.json");
        assert_eq!(lock_key(&missing), lock_key(&messy));
        // And it does NOT create the directory as a side effect.
        assert!(!missing.parent().unwrap().exists());
    }

    /// Golden anchor normalization for the four shapes `rules/paths.md`
    /// requires, on **every** host — which is only possible because
    /// `normalize_anchor` does no I/O. The verbatim `\\?\` form is the one
    /// that matters: `canonicalize` returns it on Windows, so a caller that
    /// canonicalized and one that did not would otherwise key two different
    /// locks onto the same file.
    #[test]
    fn normalize_anchor_strips_the_windows_verbatim_prefix_on_every_host() {
        for (input, expected) in [
            // Unix absolute — untouched.
            ("/Users/joker/.claude", "/Users/joker/.claude"),
            // Windows drive letter — untouched, already simple.
            (r"C:\Users\joker\.claude", r"C:\Users\joker\.claude"),
            // UNC — untouched.
            (r"\\server\share\.claude", r"\\server\share\.claude"),
            // Verbatim drive — prefix stripped.
            (r"\\?\C:\Users\joker\.claude", r"C:\Users\joker\.claude"),
            // Verbatim UNC — rewritten to the plain UNC form.
            (r"\\?\UNC\server\share\.claude", r"\\server\share\.claude"),
        ] {
            assert_eq!(
                normalize_anchor(Path::new(input)),
                PathBuf::from(expected),
                "input {input}"
            );
        }
    }

    /// The tail is re-attached outermost-first, whatever the anchor's shape.
    /// Also pure, so this runs identically on macOS, Linux and Windows.
    #[test]
    fn rejoin_tail_reattaches_components_outermost_first() {
        // `tail` is built inner-to-outer by the walk up.
        let tail = [
            std::ffi::OsString::from("settings.local.json"),
            std::ffi::OsString::from("dot-claude"),
        ];
        assert_eq!(
            rejoin_tail(Path::new("/repo"), &tail),
            PathBuf::from("/repo/dot-claude/settings.local.json")
        );
        // Nothing walked over → the anchor itself.
        assert_eq!(rejoin_tail(Path::new("/repo"), &[]), PathBuf::from("/repo"));
    }

    /// And the composed, I/O-touching function agrees with the pure pair for
    /// the tails this crate actually uses.
    #[test]
    fn lock_key_preserves_the_tail_for_every_depth() {
        let tmp = TempDir::new().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        for tail in [
            "settings.json",
            "a/settings.json",
            "a/b/c/settings.local.json",
        ] {
            let p = tmp.path().join(tail);
            let key = lock_key(&p);
            let expected: PathBuf = tail.split('/').fold(root.clone(), |acc, c| acc.join(c));
            assert_eq!(key, expected, "tail {tail}");
        }
    }

    /// A skip reports `state` as what is on disk, so it owes the same
    /// change-detection a write does.
    #[test]
    fn an_external_change_during_a_skip_forces_a_rebase_too() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("settings.json");
        std::fs::write(&p, br#"{"original":1}"#).unwrap();

        let calls = AtomicUsize::new(0);
        let path = p.clone();
        let m: Mutation<()> = mutate_settings_file(&p, move |_, _| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                std::fs::write(&path, br#"{"from_cc":true}"#).unwrap();
            }
            Ok::<_, TestError>(Change::Skip(()))
        })
        .unwrap();

        assert!(m.rebased, "a skip must notice the file moved under it");
        assert!(
            m.state.contains_key("from_cc") && !m.state.contains_key("original"),
            "skip state must describe the file as it is now, got {:?}",
            m.state
        );
        // And nothing was written.
        let after: JsonValue = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(after["from_cc"], json!(true));
    }

    #[test]
    fn read_settings_file_reports_missing_as_empty() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("nope.json");
        assert!(read_settings_file(&p).unwrap().is_empty());
    }
}
