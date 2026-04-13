//! Private helper functions for the project module.

use crate::error::ProjectError;
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
    Ok(resolved.to_string_lossy().nfc().collect::<String>())
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
    // If we recovered cwd from sessions, trust it. Otherwise, the roundtrip
    // check guards against false positives from truncated or ambiguous paths
    // (e.g. hyphens in directory names).
    let is_orphan = if recovered.is_some() {
        !Path::new(&original_path).exists()
    } else {
        let roundtrips = sanitize_path(&original_path) == sanitized_name;
        roundtrips && !Path::new(&original_path).exists()
    };

    Ok(ProjectInfo {
        sanitized_name: sanitized_name.to_string(),
        original_path,
        session_count,
        memory_file_count,
        total_size_bytes,
        last_modified,
        is_orphan,
    })
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

pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), ProjectError> {
    crate::fs_utils::copy_dir_recursive(src, dst).map_err(ProjectError::Io)
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
