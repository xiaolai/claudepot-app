//! Private helper functions for the project module.

use crate::error::ProjectError;
use crate::path_utils::simplify_windows_path;
use crate::project_sanitize::{sanitize_path, unsanitize_path, MAX_SANITIZED_LENGTH};
use crate::project_types::*;
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use unicode_normalization::UnicodeNormalization;

pub(crate) fn resolve_path(path: &str) -> Result<String, ProjectError> {
    let p = PathBuf::from(path);
    let abs = if p.is_absolute() {
        p
    } else {
        std::env::current_dir().map_err(ProjectError::Io)?.join(&p)
    };
    let resolved = if abs.exists() {
        abs.canonicalize().map_err(ProjectError::Io)?
    } else {
        abs
    };
    // Windows `canonicalize` returns `\\?\C:\...` (or `\\?\UNC\...`);
    // CC never uses the verbatim form, so strip it before sanitizing.
    // No-op on Unix.
    let simplified = simplify_windows_path(&resolved.to_string_lossy());
    Ok(simplified.nfc().collect::<String>())
}

pub(crate) fn find_project_dir_by_prefix(
    config_dir: &Path,
    sanitized_prefix: &str,
) -> Result<Option<PathBuf>, ProjectError> {
    let projects_dir = config_dir.join("projects");
    if !projects_dir.exists() {
        return Ok(None);
    }

    let prefix = if sanitized_prefix.len() > MAX_SANITIZED_LENGTH {
        &sanitized_prefix[..MAX_SANITIZED_LENGTH]
    } else {
        sanitized_prefix
    };

    let mut matches = Vec::new();
    for entry in fs::read_dir(&projects_dir).map_err(ProjectError::Io)? {
        let entry = entry.map_err(ProjectError::Io)?;
        if !entry.file_type().map_err(ProjectError::Io)?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(prefix) {
            matches.push(entry.path());
        }
    }

    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches.remove(0))),
        _ => Err(ProjectError::Ambiguous(format!(
            "multiple CC project dirs match prefix '{}'",
            prefix
        ))),
    }
}

pub(crate) fn compute_project_info(
    dir: &Path,
    sanitized_name: &str,
) -> Result<ProjectInfo, ProjectError> {
    // Prefer the authoritative `cwd` field from any session.jsonl. Fall back to
    // the lossy unsanitize_path only when no session metadata is available.
    let recovered = recover_cwd_from_sessions(dir);
    let original_path = recovered
        .clone()
        .unwrap_or_else(|| unsanitize_path(sanitized_name));
    let session_count = count_files_with_ext(dir, "jsonl");
    let memory_dir = dir.join("memory");
    let memory_file_count = if memory_dir.exists() {
        count_files_with_ext(&memory_dir, "md")
    } else {
        0
    };
    let total_size_bytes = dir_size(dir);
    let last_modified = most_recent_mtime(dir);

    // Empty-dir heuristic: no sessions, no memory, and below one
    // filesystem block (directory entries themselves take space, but
    // a truly abandoned CC project dir stays under ~4 KiB). Empty dirs
    // are always safe to remove regardless of path reachability.
    let is_empty = session_count == 0 && memory_file_count == 0 && total_size_bytes < 4096;

    // Reachability — can we definitively stat the source? `try_exists()`
    // distinguishes NotFound (reachable, absent) from other I/O errors
    // (unreachable). Then: a path under a known mount-root whose mount
    // point itself is absent is unreachable, NOT absent.
    let reachability = classify_reachability(&original_path);
    let is_reachable = matches!(
        reachability,
        PathReachability::Exists | PathReachability::Absent
    );
    let source_confirmed_absent = matches!(reachability, PathReachability::Absent);

    // Classification for `is_orphan` — a candidate for `project clean`:
    //   1. Empty project dir: always. We can't misidentify the source
    //      because there's nothing to lose.
    //   2. Source confirmed absent AND roundtrip-safe: source was either
    //      recovered from session.jsonl (authoritative) or the sanitized
    //      name is a clean bijection. Unreachable paths never qualify.
    let is_orphan = if is_empty {
        true
    } else if !is_reachable {
        false
    } else if recovered.is_some() {
        source_confirmed_absent
    } else {
        let roundtrips = sanitize_path(&original_path) == sanitized_name;
        roundtrips && source_confirmed_absent
    };

    Ok(ProjectInfo {
        sanitized_name: sanitized_name.to_string(),
        original_path,
        session_count,
        memory_file_count,
        total_size_bytes,
        last_modified,
        is_orphan,
        is_reachable,
        is_empty,
    })
}

/// Three-state reachability of an arbitrary path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathReachability {
    /// Stat succeeded and the path is present.
    Exists,
    /// Stat succeeded and the path is absent; callers can safely treat
    /// this as "deleted".
    Absent,
    /// Stat failed (permission denied, EIO) or the path sits under a
    /// known removable-mount root whose mount point is missing. The
    /// path's status is genuinely unknown — treating it as absent
    /// would risk deleting data that's merely unplugged.
    Unreachable,
}

pub(crate) fn classify_reachability(original_path: &str) -> PathReachability {
    if original_path.is_empty() {
        return PathReachability::Unreachable;
    }

    if is_under_absent_mount(original_path) {
        return PathReachability::Unreachable;
    }

    match Path::new(original_path).try_exists() {
        Ok(true) => PathReachability::Exists,
        Ok(false) => PathReachability::Absent,
        // Permission-denied / EIO / any other stat error. Path might or
        // might not exist; we can't tell, so refuse to classify as absent.
        Err(_) => PathReachability::Unreachable,
    }
}

/// Is `path` anchored under a removable-mount root whose mount point
/// itself is gone? If so, the mount is likely unplugged (external SSD,
/// network share) and the path's absence tells us nothing about whether
/// the source data still exists on that volume.
fn is_under_absent_mount(path: &str) -> bool {
    #[cfg(unix)]
    {
        // macOS: `/Volumes/<name>`; Linux: `/mnt/<name>`, `/media/<name>`,
        // `/run/media/<user>/<name>`. Treat each as: strip root, the
        // first segment is the mount dir.
        const MOUNT_ROOTS: &[&str] = &["/Volumes/", "/mnt/", "/media/", "/run/media/"];
        for root in MOUNT_ROOTS {
            if let Some(rest) = path.strip_prefix(root) {
                let first = rest.split('/').next().unwrap_or("");
                if first.is_empty() {
                    return false;
                }
                // `/run/media/<user>/<name>` — descend one more level.
                let mount_point = if *root == "/run/media/" {
                    let mut it = rest.splitn(3, '/');
                    let (Some(u), Some(n)) = (it.next(), it.next()) else {
                        return false;
                    };
                    if u.is_empty() || n.is_empty() {
                        return false;
                    }
                    format!("{root}{u}/{n}")
                } else {
                    format!("{root}{first}")
                };
                return !Path::new(&mount_point).exists();
            }
        }
    }
    #[cfg(windows)]
    {
        // UNC: `\\host\share\...`. The mount point is `\\host\share`.
        if let Some(rest) = path.strip_prefix("\\\\") {
            let mut parts = rest.splitn(3, '\\');
            let (Some(host), Some(share)) = (parts.next(), parts.next()) else {
                return false;
            };
            if host.is_empty() || share.is_empty() {
                return false;
            }
            let unc_root = format!("\\\\{host}\\{share}");
            return !Path::new(&unc_root).exists();
        }
    }
    let _ = path;
    false
}

/// Recover the project's original cwd from any session.jsonl's `cwd` field.
/// CC writes `cwd` into every session entry, so this is authoritative and
/// survives lossy sanitization (hyphens, long paths, unicode).
pub(crate) fn recover_cwd_from_sessions(dir: &Path) -> Option<String> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".jsonl") {
            continue;
        }
        let Ok(file) = fs::File::open(entry.path()) else {
            continue;
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(cwd) = val.get("cwd").and_then(|v| v.as_str()) {
                if !cwd.is_empty() {
                    return Some(cwd.to_string());
                }
            }
        }
    }
    None
}

pub(crate) fn list_sessions(dir: &Path) -> Result<Vec<SessionInfo>, ProjectError> {
    let mut sessions = Vec::new();
    for entry in fs::read_dir(dir).map_err(ProjectError::Io)? {
        let entry = entry.map_err(ProjectError::Io)?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".jsonl") {
            continue;
        }
        let session_id = name.trim_end_matches(".jsonl").to_string();
        let meta = entry.metadata().map_err(ProjectError::Io)?;
        sessions.push(SessionInfo {
            session_id,
            file_size: meta.len(),
            last_modified: meta.modified().ok(),
        });
    }
    sessions.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));
    Ok(sessions)
}

pub(crate) fn list_memory_files(dir: &Path) -> Result<Vec<String>, ProjectError> {
    let memory_dir = dir.join("memory");
    if !memory_dir.exists() {
        return Ok(vec![]);
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(&memory_dir).map_err(ProjectError::Io)? {
        let entry = entry.map_err(ProjectError::Io)?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".md") {
            files.push(name);
        }
    }
    files.sort();
    Ok(files)
}

pub(crate) fn count_files_with_ext(dir: &Path, ext: &str) -> usize {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map(|x| x == ext).unwrap_or(false))
                .count()
        })
        .unwrap_or(0)
}

pub(crate) fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.is_file() {
                total += meta.len();
            } else if meta.is_dir() {
                total += dir_size(&entry.path());
            }
        }
    }
    total
}

pub(crate) fn most_recent_mtime(dir: &Path) -> Option<SystemTime> {
    let mut latest: Option<SystemTime> = None;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let mtime = meta.modified().ok();
            if meta.is_dir() {
                let sub = most_recent_mtime(&entry.path());
                if sub > latest {
                    latest = sub;
                }
            }
            if mtime > latest {
                latest = mtime;
            }
        }
    }
    latest
}

pub(crate) fn is_claude_running_in(dir: &str) -> bool {
    use sysinfo::System;
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    for proc in sys.processes().values() {
        let name = proc.name().to_string_lossy();
        if name.contains("claude") || name.contains("Claude") {
            let cwd = proc.cwd().map(|p| p.to_string_lossy().to_string());
            if cwd.as_deref() == Some(dir) {
                return true;
            }
        }
    }
    false
}

/// Composite live-session detector per spec §5.
///
/// Two-signal rule: either lsof / process scan is a direct-kernel
/// truth signal, OR heartbeat (mtime) combined with a confirming
/// kernel signal. Heartbeat alone is NOT sufficient: fresh test
/// fixtures and benign writers (backup tools, IDEs) all produce
/// recent mtimes without implying a live CC session.
///
/// Signals:
///   1. lsof: any process holding a file under the CC project dir
///      open — direct kernel truth.
///   2. Process scan: claude-named process with matching cwd —
///      direct kernel truth.
///   3. Heartbeat: newest `*.jsonl` mtime inside the CC project dir.
///      Used to escalate the lsof check from optional to mandatory:
///      if mtime is recent, we require lsof/process confirmation
///      before declaring live. If lsof is unavailable and mtime is
///      recent, report live on age alone as a conservative fallback.
///
/// Returns `true` iff a live session is plausibly present. `--force`
/// callers should bypass this check, not ignore it.
pub fn detect_live_session(
    cc_project_dir: &Path,
    project_cwd: &str,
    heartbeat_window_secs: u64,
) -> bool {
    // Kernel signals (authoritative).
    if lsof_sees_open_file(cc_project_dir) {
        tracing::debug!(dir = ?cc_project_dir, "lsof signal: open file detected");
        return true;
    }
    if is_claude_running_in(project_cwd) {
        tracing::debug!(cwd = %project_cwd, "process scan: claude cwd match");
        return true;
    }
    // Heartbeat as a conservative fallback: only use if the mtime is
    // within the window AND at least one weaker signal confirms. On
    // systems without lsof, fall back to mtime-alone after logging.
    if let Some(newest_mtime) = newest_jsonl_mtime(cc_project_dir) {
        let age = SystemTime::now()
            .duration_since(newest_mtime)
            .map(|d| d.as_secs())
            .unwrap_or(u64::MAX);
        if age <= heartbeat_window_secs && !lsof_available() {
            tracing::debug!(
                age_secs = age,
                "heartbeat only — lsof unavailable, treating as live"
            );
            return true;
        }
    }
    false
}

/// Detect whether `lsof` is available on PATH.
fn lsof_available() -> bool {
    std::process::Command::new("lsof")
        .arg("-v")
        .output()
        .map(|o| o.status.success() || o.stderr.is_empty() == false)
        .unwrap_or(false)
}

/// Return the most recent mtime among `.jsonl` files at any depth
/// under the project dir. None if the dir is missing or empty.
fn newest_jsonl_mtime(project_dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    walk_jsonl_mtime(project_dir, &mut newest);
    newest
}

fn walk_jsonl_mtime(dir: &Path, newest: &mut Option<SystemTime>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            walk_jsonl_mtime(&path, newest);
        } else if ft.is_file()
            && path
                .extension()
                .map(|e| e == "jsonl")
                .unwrap_or(false)
        {
            if let Ok(meta) = entry.metadata() {
                if let Ok(mtime) = meta.modified() {
                    if newest.map(|n| mtime > n).unwrap_or(true) {
                        *newest = Some(mtime);
                    }
                }
            }
        }
    }
}

/// Crate-public wrapper so `project::move_project` can probe a
/// project cwd directly (spec §5 secondary signal).
pub fn lsof_sees_open_file_pub(dir: &Path) -> bool {
    lsof_sees_open_file(dir)
}

/// Use `lsof` to check if any process has a file under the given dir
/// open. On platforms without `lsof` (or if it fails) returns false —
/// heartbeat and process-scan signals remain the fallback.
fn lsof_sees_open_file(dir: &Path) -> bool {
    let dir_str = dir.to_string_lossy();
    let output = std::process::Command::new("lsof")
        .args(["+D", &dir_str])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            // Any output line means an open handle exists.
            !out.stdout.is_empty()
        }
        _ => false,
    }
}

pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), ProjectError> {
    crate::fs_utils::copy_dir_recursive(src, dst).map_err(ProjectError::Io)
}

/// Public wrapper for `merge_project_dirs`, exposed for P8's memory-dir
/// merge case.
pub fn merge_project_dirs_pub(src: &Path, dst: &Path) -> Result<(), ProjectError> {
    merge_project_dirs(src, dst)
}

pub(crate) fn merge_project_dirs(src: &Path, dst: &Path) -> Result<(), ProjectError> {
    for entry in fs::read_dir(src).map_err(ProjectError::Io)? {
        let entry = entry.map_err(ProjectError::Io)?;
        let target = dst.join(entry.file_name());
        if entry.file_type().map_err(ProjectError::Io)?.is_dir() {
            if !target.exists() {
                fs::create_dir_all(&target).map_err(ProjectError::Io)?;
            }
            merge_project_dirs(&entry.path(), &target)?;
        } else if !target.exists() {
            fs::copy(entry.path(), &target).map_err(ProjectError::Io)?;
        }
    }
    Ok(())
}

pub(crate) fn rewrite_history(
    history_path: &Path,
    old_path: &str,
    new_path: &str,
) -> Result<usize, ProjectError> {
    let tmp = tempfile::NamedTempFile::new_in(history_path.parent().unwrap_or(Path::new(".")))
        .map_err(ProjectError::Io)?;

    let reader = BufReader::new(fs::File::open(history_path).map_err(ProjectError::Io)?);
    let mut writer = BufWriter::new(&tmp);
    let mut count = 0;

    // Pre-check against the JSON-escaped form of old_path to avoid parsing
    // lines that obviously don't reference it. On Windows, raw paths contain
    // backslashes that appear as double-escaped sequences in the file, so
    // `line.contains(old_path)` on the raw string misses them. Use the
    // JSON-serialized form of old_path for correct escape matching.
    let old_needle = serde_json::to_string(old_path).unwrap_or_else(|_| format!("\"{old_path}\""));
    // Strip enclosing quotes: we want the needle to be just the escaped path,
    // not a fully-quoted JSON string, so it matches inside the surrounding
    // `"project":"…"` context.
    let old_needle = old_needle.trim_matches('"');

    for line in reader.lines() {
        let line = line.map_err(ProjectError::Io)?;
        if line.contains(old_needle) {
            if let Ok(mut entry) = serde_json::from_str::<serde_json::Value>(&line) {
                if let Some(proj) = entry.get_mut("project") {
                    if proj.as_str() == Some(old_path) {
                        *proj = serde_json::Value::String(new_path.to_string());
                        count += 1;
                    }
                }
                writeln!(writer, "{}", serde_json::to_string(&entry).unwrap_or(line))
                    .map_err(ProjectError::Io)?;
            } else {
                writeln!(writer, "{}", line).map_err(ProjectError::Io)?;
            }
        } else {
            writeln!(writer, "{}", line).map_err(ProjectError::Io)?;
        }
    }

    drop(writer);
    tmp.persist(history_path)
        .map_err(|e| ProjectError::Io(e.error))?;
    Ok(count)
}

pub(crate) fn estimate_history_matches(config_dir: &Path, old_path: &str) -> usize {
    let history_path = config_dir.join("history.jsonl");
    if !history_path.exists() {
        return 0;
    }
    fs::File::open(&history_path)
        .map(|f| {
            BufReader::new(f)
                .lines()
                .map_while(Result::ok)
                .filter(|l| l.contains(old_path))
                .count()
        })
        .unwrap_or(0)
}
