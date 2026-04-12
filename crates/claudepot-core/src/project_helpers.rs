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
        std::env::current_dir()
            .map_err(ProjectError::Io)?
            .join(&p)
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
    let original_path = unsanitize_path(sanitized_name);
    let session_count = count_files_with_ext(dir, "jsonl");
    let memory_dir = dir.join("memory");
    let memory_file_count = if memory_dir.exists() {
        count_files_with_ext(&memory_dir, "md")
    } else {
        0
    };
    let total_size_bytes = dir_size(dir);
    let last_modified = most_recent_mtime(dir);
    let roundtrips = sanitize_path(&original_path) == sanitized_name;
    let is_orphan = roundtrips && !Path::new(&original_path).exists();

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
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|x| x == ext)
                        .unwrap_or(false)
                })
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

    for (_pid, proc) in sys.processes() {
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
    let tmp = tempfile::NamedTempFile::new_in(
        history_path.parent().unwrap_or(Path::new(".")),
    )
    .map_err(ProjectError::Io)?;

    let reader = BufReader::new(fs::File::open(history_path).map_err(ProjectError::Io)?);
    let mut writer = BufWriter::new(&tmp);
    let mut count = 0;

    for line in reader.lines() {
        let line = line.map_err(ProjectError::Io)?;
        if line.contains(old_path) {
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

pub(crate) fn compute_dry_run_plan(
    config_dir: &Path,
    old_norm: &str,
    _new_norm: &str,
    old_san: &str,
    new_san: &str,
    scenario: &super::project::MoveScenario,
) -> Result<DryRunPlan, ProjectError> {
    let cc_old = config_dir.join("projects").join(old_san);
    let cc_new = config_dir.join("projects").join(new_san);

    let (session_count, cc_dir_size) = if cc_old.exists() {
        (count_files_with_ext(&cc_old, "jsonl"), dir_size(&cc_old))
    } else {
        (0, 0)
    };

    let estimated_history_lines = estimate_history_matches(config_dir, old_norm);

    let conflict = if old_san != new_san && cc_new.exists() {
        let is_empty = fs::read_dir(&cc_new)
            .map(|mut d| d.next().is_none())
            .unwrap_or(true);
        if is_empty {
            None
        } else {
            Some(format!(
                "CC dir already exists at '{}' (non-empty). Use --merge or --overwrite.",
                new_san
            ))
        }
    } else {
        None
    };

    Ok(DryRunPlan {
        would_move_dir: *scenario == super::project::MoveScenario::MoveAndUpdate,
        old_cc_dir: old_san.to_string(),
        new_cc_dir: new_san.to_string(),
        session_count,
        cc_dir_size,
        estimated_history_lines,
        conflict,
    })
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
                .filter_map(|l| l.ok())
                .filter(|l| l.contains(old_path))
                .count()
        })
        .unwrap_or(0)
}

pub(crate) fn format_dry_run_plan(plan: &DryRunPlan, old_norm: &str, new_norm: &str) -> String {
    let mut out = String::from("Dry run \u{2014} no changes will be made.\n\nWould:\n");

    let mut step = 1;
    if plan.would_move_dir {
        out.push_str(&format!(
            "  {}. Move {} \u{2192} {}\n",
            step, old_norm, new_norm
        ));
        step += 1;
    }

    if plan.old_cc_dir != plan.new_cc_dir {
        out.push_str(&format!(
            "  {}. Rename CC dir: {} \u{2192} {}\n     ({} sessions, {})\n",
            step,
            plan.old_cc_dir,
            plan.new_cc_dir,
            plan.session_count,
            format_size(plan.cc_dir_size)
        ));
        step += 1;
    }

    if plan.estimated_history_lines > 0 {
        out.push_str(&format!(
            "  {}. Rewrite ~{} history.jsonl entries\n",
            step, plan.estimated_history_lines
        ));
    }

    if let Some(ref conflict) = plan.conflict {
        out.push_str(&format!("\nConflict: {}\n", conflict));
    } else {
        out.push_str("\nNo conflicts detected.\n");
    }

    out
}

/// Format bytes as human-readable size.
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
