use crate::error::ProjectError;
use serde::Serialize;
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

// ---------------------------------------------------------------------------
// Path sanitization (mirrors CC's sessionStoragePortable.ts:311-319)
// ---------------------------------------------------------------------------

const MAX_SANITIZED_LENGTH: usize = 200;

/// Replicate CC's `sanitizePath`. Non-alphanumeric ASCII chars become `-`.
/// Paths longer than 200 chars get a djb2 hash suffix.
pub fn sanitize_path(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    if sanitized.len() <= MAX_SANITIZED_LENGTH {
        sanitized
    } else {
        let hash = djb2_hash(name);
        format!("{}-{}", &sanitized[..MAX_SANITIZED_LENGTH], hash)
    }
}

/// Best-effort reverse of `sanitize_path`. Lossy: hyphens could have been
/// any non-alphanumeric char. Used for display only.
pub fn unsanitize_path(sanitized: &str) -> String {
    // Leading `-` was a `/` (Unix) or drive separator (Windows).
    // Remaining `-` are path separators — this is wrong for names containing
    // hyphens/underscores/spaces, but it's the best we can do.
    sanitized.replace('-', "/")
}

fn djb2_hash(s: &str) -> String {
    let mut hash: u32 = 5381;
    for byte in s.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(byte as u32);
    }
    format_radix(hash, 36)
}

fn format_radix(mut x: u32, radix: u32) -> String {
    if x == 0 {
        return "0".to_string();
    }
    let mut result = Vec::new();
    while x > 0 {
        let digit = (x % radix) as u8;
        let ch = if digit < 10 {
            b'0' + digit
        } else {
            b'a' + digit - 10
        };
        result.push(ch);
        x /= radix;
    }
    result.reverse();
    String::from_utf8(result).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ProjectInfo {
    pub sanitized_name: String,
    pub original_path: String,
    pub session_count: usize,
    pub memory_file_count: usize,
    pub total_size_bytes: u64,
    pub last_modified: Option<SystemTime>,
    pub is_orphan: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectDetail {
    pub info: ProjectInfo,
    pub sessions: Vec<SessionInfo>,
    pub memory_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub file_size: u64,
    pub last_modified: Option<SystemTime>,
}

pub struct MoveArgs {
    pub old_path: PathBuf,
    pub new_path: PathBuf,
    pub config_dir: PathBuf,
    pub no_move: bool,
    pub merge: bool,
    pub overwrite: bool,
    pub force: bool,
    pub dry_run: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct MoveResult {
    pub actual_dir_moved: bool,
    pub cc_dir_renamed: bool,
    pub old_sanitized: Option<String>,
    pub new_sanitized: Option<String>,
    pub history_lines_updated: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct CleanResult {
    pub orphans_found: usize,
    pub orphans_removed: usize,
    pub bytes_freed: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DryRunPlan {
    pub would_move_dir: bool,
    pub old_cc_dir: String,
    pub new_cc_dir: String,
    pub session_count: usize,
    pub cc_dir_size: u64,
    pub estimated_history_lines: usize,
    pub conflict: Option<String>,
}

// ---------------------------------------------------------------------------
// list_projects
// ---------------------------------------------------------------------------

pub fn list_projects(config_dir: &Path) -> Result<Vec<ProjectInfo>, ProjectError> {
    let projects_dir = config_dir.join("projects");
    if !projects_dir.exists() {
        return Ok(vec![]);
    }

    let mut projects = Vec::new();
    for entry in fs::read_dir(&projects_dir).map_err(ProjectError::Io)? {
        let entry = entry.map_err(ProjectError::Io)?;
        let ft = entry.file_type().map_err(ProjectError::Io)?;
        if !ft.is_dir() {
            continue;
        }

        let sanitized_name = entry.file_name().to_string_lossy().to_string();
        let original_path = unsanitize_path(&sanitized_name);

        let session_count = count_files_with_ext(&entry.path(), "jsonl");
        let memory_dir = entry.path().join("memory");
        let memory_file_count = if memory_dir.exists() {
            count_files_with_ext(&memory_dir, "md")
        } else {
            0
        };
        let total_size_bytes = dir_size(&entry.path());
        let last_modified = most_recent_mtime(&entry.path());
        let is_orphan = !Path::new(&original_path).exists();

        projects.push(ProjectInfo {
            sanitized_name,
            original_path,
            session_count,
            memory_file_count,
            total_size_bytes,
            last_modified,
            is_orphan,
        });
    }

    projects.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));
    Ok(projects)
}

// ---------------------------------------------------------------------------
// show_project
// ---------------------------------------------------------------------------

pub fn show_project(
    config_dir: &Path,
    path: &str,
) -> Result<ProjectDetail, ProjectError> {
    let resolved = resolve_path(path)?;
    let sanitized = sanitize_path(&resolved);
    let project_dir = config_dir.join("projects").join(&sanitized);

    // If exact match doesn't exist, try prefix scan (for long-path hash mismatches)
    let project_dir = if project_dir.exists() {
        project_dir
    } else {
        find_project_dir_by_prefix(config_dir, &sanitized)?
            .ok_or_else(|| ProjectError::NotFound(path.to_string()))?
    };

    let sanitized_name = project_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let info = compute_project_info(&project_dir, &sanitized_name)?;
    let sessions = list_sessions(&project_dir)?;
    let memory_files = list_memory_files(&project_dir)?;

    Ok(ProjectDetail {
        info,
        sessions,
        memory_files,
    })
}

// ---------------------------------------------------------------------------
// move_project
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum MoveScenario {
    StateOnly,
    MoveAndUpdate,
    AlreadyMoved,
}

pub fn move_project(args: &MoveArgs) -> Result<MoveResult, ProjectError> {
    tracing::info!(old = ?args.old_path, new = ?args.new_path, "starting project move");

    // Phase 1: Validate
    let old_str = args.old_path.to_str().ok_or_else(|| {
        ProjectError::Ambiguous("old path contains invalid UTF-8".to_string())
    })?;
    let new_str = args.new_path.to_str().ok_or_else(|| {
        ProjectError::Ambiguous("new path contains invalid UTF-8".to_string())
    })?;
    let old_norm = resolve_path(old_str)?;
    let new_norm = resolve_path(new_str)?;

    if old_norm == new_norm {
        return Err(ProjectError::SamePath);
    }

    let old_san = sanitize_path(&old_norm);
    let new_san = sanitize_path(&new_norm);

    // Phase 2: Detect scenario
    let old_exists = Path::new(&old_norm).exists();
    let new_exists = Path::new(&new_norm).exists();

    let scenario = if args.no_move {
        MoveScenario::StateOnly
    } else {
        match (old_exists, new_exists) {
            (true, false) => MoveScenario::MoveAndUpdate,
            (false, true) => MoveScenario::AlreadyMoved,
            (true, true) => {
                return Err(ProjectError::Ambiguous(
                    "both old and new paths exist on disk".to_string(),
                ))
            }
            (false, false) => {
                return Err(ProjectError::Ambiguous(
                    "neither old nor new path exists on disk".to_string(),
                ))
            }
        }
    };

    // Dry run: compute plan and return
    if args.dry_run {
        let plan = compute_dry_run_plan(
            &args.config_dir,
            &old_norm,
            &new_norm,
            &old_san,
            &new_san,
            &scenario,
        )?;
        // Return a MoveResult with warnings containing the plan description
        return Ok(MoveResult {
            warnings: vec![format_dry_run_plan(&plan, &old_norm, &new_norm)],
            ..Default::default()
        });
    }

    let mut result = MoveResult::default();

    // Phase 3: Move actual directory
    if scenario == MoveScenario::MoveAndUpdate {
        if !args.force && is_claude_running_in(&old_norm) {
            return Err(ProjectError::ClaudeRunning(old_norm.clone()));
        }

        if let Some(parent) = Path::new(&new_norm).parent() {
            fs::create_dir_all(parent).map_err(ProjectError::Io)?;
        }

        match fs::rename(&old_norm, &new_norm) {
            Ok(()) => {}
            #[cfg(unix)]
            Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
                copy_dir_recursive(Path::new(&old_norm), Path::new(&new_norm))?;
                fs::remove_dir_all(&old_norm).map_err(ProjectError::Io)?;
            }
            Err(e) => return Err(ProjectError::Io(e)),
        }
        result.actual_dir_moved = true;
    }

    // Phase 4: Rename CC project directory
    result.old_sanitized = Some(old_san.clone());
    result.new_sanitized = Some(new_san.clone());
    if old_san != new_san {
        let projects_base = args.config_dir.join("projects");
        let cc_old = projects_base.join(&old_san);
        let cc_new = projects_base.join(&new_san);

        // Defense-in-depth: ensure paths stay within projects/
        if !cc_old.starts_with(&projects_base) || !cc_new.starts_with(&projects_base) {
            return Err(ProjectError::Ambiguous(
                "sanitized path escapes projects directory".to_string(),
            ));
        }

        if cc_old.exists() {
            if cc_new.exists() {
                let new_is_empty = fs::read_dir(&cc_new)
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(false);

                if new_is_empty {
                    fs::remove_dir(&cc_new).map_err(ProjectError::Io)?;
                    fs::rename(&cc_old, &cc_new).map_err(ProjectError::Io)?;
                    result.cc_dir_renamed = true;
                } else if args.merge {
                    merge_project_dirs(&cc_old, &cc_new)?;
                    fs::remove_dir_all(&cc_old).map_err(ProjectError::Io)?;
                    result.cc_dir_renamed = true;
                } else if args.overwrite {
                    fs::remove_dir_all(&cc_new).map_err(ProjectError::Io)?;
                    fs::rename(&cc_old, &cc_new).map_err(ProjectError::Io)?;
                    result.cc_dir_renamed = true;
                } else {
                    result.warnings.push(
                        "CC project data exists at both old and new paths. \
                         Use --merge or --overwrite to resolve."
                            .to_string(),
                    );
                }
            } else {
                fs::rename(&cc_old, &cc_new).map_err(ProjectError::Io)?;
                result.cc_dir_renamed = true;
            }
        }
    }

    // Phase 5: Rewrite history.jsonl
    let history_path = args.config_dir.join("history.jsonl");
    if history_path.exists() {
        tracing::debug!("rewriting history.jsonl");
        result.history_lines_updated = rewrite_history(&history_path, &old_norm, &new_norm)?;
    }

    tracing::info!(
        moved = result.actual_dir_moved,
        renamed = result.cc_dir_renamed,
        history = result.history_lines_updated,
        "project move complete"
    );
    Ok(result)
}

// ---------------------------------------------------------------------------
// clean_orphans
// ---------------------------------------------------------------------------

pub fn clean_orphans(
    config_dir: &Path,
    dry_run: bool,
) -> Result<(CleanResult, Vec<ProjectInfo>), ProjectError> {
    let projects = list_projects(config_dir)?;
    let orphans: Vec<ProjectInfo> = projects.into_iter().filter(|p| p.is_orphan).collect();

    let mut result = CleanResult {
        orphans_found: orphans.len(),
        orphans_removed: 0,
        bytes_freed: 0,
    };

    if !dry_run {
        for orphan in &orphans {
            let dir = config_dir.join("projects").join(&orphan.sanitized_name);
            // Re-verify orphan status to guard against TOCTOU:
            // the source path could have been created between the scan and now.
            if dir.exists() && !Path::new(&orphan.original_path).exists() {
                result.bytes_freed += orphan.total_size_bytes;
                fs::remove_dir_all(&dir).map_err(ProjectError::Io)?;
                result.orphans_removed += 1;
            }
        }
    }

    Ok((result, orphans))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_path(path: &str) -> Result<String, ProjectError> {
    let p = PathBuf::from(path);
    let abs = if p.is_absolute() {
        p
    } else {
        std::env::current_dir()
            .map_err(ProjectError::Io)?
            .join(&p)
    };
    // Normalize: resolve symlinks if the path exists, otherwise just canonicalize components
    let resolved = if abs.exists() {
        abs.canonicalize().map_err(ProjectError::Io)?
    } else {
        abs
    };
    Ok(resolved.to_string_lossy().to_string())
}

fn find_project_dir_by_prefix(
    config_dir: &Path,
    sanitized_prefix: &str,
) -> Result<Option<PathBuf>, ProjectError> {
    let projects_dir = config_dir.join("projects");
    if !projects_dir.exists() {
        return Ok(None);
    }

    // Prefix-truncated at MAX_SANITIZED_LENGTH for long paths
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

fn compute_project_info(
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
    let is_orphan = !Path::new(&original_path).exists();

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

fn list_sessions(dir: &Path) -> Result<Vec<SessionInfo>, ProjectError> {
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

fn list_memory_files(dir: &Path) -> Result<Vec<String>, ProjectError> {
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

fn count_files_with_ext(dir: &Path, ext: &str) -> usize {
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

fn dir_size(dir: &Path) -> u64 {
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

fn most_recent_mtime(dir: &Path) -> Option<SystemTime> {
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

fn is_claude_running_in(dir: &str) -> bool {
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

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), ProjectError> {
    crate::fs_utils::copy_dir_recursive(src, dst).map_err(ProjectError::Io)
}

fn merge_project_dirs(src: &Path, dst: &Path) -> Result<(), ProjectError> {
    for entry in fs::read_dir(src).map_err(ProjectError::Io)? {
        let entry = entry.map_err(ProjectError::Io)?;
        let target = dst.join(entry.file_name());
        if entry.file_type().map_err(ProjectError::Io)?.is_dir() {
            if !target.exists() {
                fs::create_dir_all(&target).map_err(ProjectError::Io)?;
            }
            merge_project_dirs(&entry.path(), &target)?;
        } else if !target.exists() {
            // Only copy files that don't already exist in destination
            fs::copy(entry.path(), &target).map_err(ProjectError::Io)?;
        }
    }
    Ok(())
}

fn rewrite_history(
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

fn compute_dry_run_plan(
    config_dir: &Path,
    old_norm: &str,
    _new_norm: &str,
    old_san: &str,
    new_san: &str,
    scenario: &MoveScenario,
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
        would_move_dir: *scenario == MoveScenario::MoveAndUpdate,
        old_cc_dir: old_san.to_string(),
        new_cc_dir: new_san.to_string(),
        session_count,
        cc_dir_size,
        estimated_history_lines,
        conflict,
    })
}

fn estimate_history_matches(config_dir: &Path, old_path: &str) -> usize {
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

fn format_dry_run_plan(plan: &DryRunPlan, old_norm: &str, new_norm: &str) -> String {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_unix_path() {
        assert_eq!(
            sanitize_path("/Users/joker/github/xiaolai/myprojects/kannon"),
            "-Users-joker-github-xiaolai-myprojects-kannon"
        );
    }

    #[test]
    fn test_sanitize_windows_path() {
        assert_eq!(
            sanitize_path("C:\\Users\\joker\\project"),
            "C--Users-joker-project"
        );
    }

    #[test]
    fn test_sanitize_preserves_alphanumeric() {
        assert_eq!(sanitize_path("abc123"), "abc123");
    }

    #[test]
    fn test_sanitize_replaces_special_chars() {
        assert_eq!(sanitize_path("/a.b_c-d"), "-a-b-c-d");
    }

    #[test]
    fn test_sanitize_long_path_with_hash() {
        let long_path = "/".to_string() + &"a".repeat(250);
        let result = sanitize_path(&long_path);
        // Should be 200 chars + '-' + hash
        assert!(result.len() > MAX_SANITIZED_LENGTH);
        assert!(result.starts_with("-"));
        // The first 200 chars should be from the sanitized path
        let prefix = &result[..MAX_SANITIZED_LENGTH];
        assert!(prefix.chars().all(|c| c == '-' || c == 'a'));
    }

    #[test]
    fn test_sanitize_unicode_path() {
        // Unicode chars are non-alphanumeric, should become `-`
        assert_eq!(sanitize_path("/tmp/\u{00e9}l\u{00e8}ve"), "-tmp--l-ve");
    }

    #[test]
    fn test_unsanitize_roundtrip_simple() {
        let original = "/Users/joker/project";
        let sanitized = sanitize_path(original);
        let unsanitized = unsanitize_path(&sanitized);
        assert_eq!(unsanitized, original);
    }

    #[test]
    fn test_unsanitize_lossy() {
        // Hyphens and underscores both become `-`, so unsanitize is lossy
        let sanitized = sanitize_path("/my-project");
        let unsanitized = unsanitize_path(&sanitized);
        // Original was /my-project, sanitized to -my-project, unsanitized to /my/project
        assert_eq!(unsanitized, "/my/project");
    }

    #[test]
    fn test_djb2_hash_deterministic() {
        let h1 = djb2_hash("test");
        let h2 = djb2_hash("test");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_djb2_hash_different_inputs() {
        let h1 = djb2_hash("abc");
        let h2 = djb2_hash("def");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_format_radix_base36() {
        assert_eq!(format_radix(0, 36), "0");
        assert_eq!(format_radix(35, 36), "z");
        assert_eq!(format_radix(36, 36), "10");
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1048576), "1.0 MB");
        assert_eq!(format_size(1073741824), "1.0 GB");
    }

    #[test]
    fn test_list_projects_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path();
        // No projects/ dir at all
        let result = list_projects(config_dir).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_list_projects_with_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let projects_dir = tmp.path().join("projects");
        fs::create_dir(&projects_dir).unwrap();

        // Create a fake project
        let proj = projects_dir.join("-tmp-myproject");
        fs::create_dir(&proj).unwrap();
        fs::write(proj.join("abc.jsonl"), "{}").unwrap();
        fs::write(proj.join("def.jsonl"), "{}").unwrap();

        let memory_dir = proj.join("memory");
        fs::create_dir(&memory_dir).unwrap();
        fs::write(memory_dir.join("MEMORY.md"), "# mem").unwrap();

        let result = list_projects(tmp.path()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].sanitized_name, "-tmp-myproject");
        assert_eq!(result[0].session_count, 2);
        assert_eq!(result[0].memory_file_count, 1);
        assert!(result[0].is_orphan); // /tmp/myproject likely doesn't exist
    }

    #[test]
    fn test_show_project_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let projects_dir = tmp.path().join("projects");
        fs::create_dir(&projects_dir).unwrap();

        let result = show_project(tmp.path(), "/nonexistent/path");
        assert!(result.is_err());
    }

    #[test]
    fn test_move_project_same_path() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("myproject");
        fs::create_dir(&src).unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: src.clone(),
            config_dir: tmp.path().to_path_buf(),
            no_move: false,
            merge: false,
            overwrite: false,
            force: false,
            dry_run: false,
        };

        let result = move_project(&args);
        assert!(matches!(result, Err(ProjectError::SamePath)));
    }

    #[test]
    fn test_move_project_renames_cc_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // Canonicalize to handle macOS /tmp -> /private/tmp symlink
        let base = tmp.path().canonicalize().unwrap();

        // Create source directory
        let src = base.join("old");
        fs::create_dir(&src).unwrap();

        // Create CC project dir for old path (using canonical path)
        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();
        let old_san = sanitize_path(&src.to_string_lossy());
        let cc_old = projects_dir.join(&old_san);
        fs::create_dir(&cc_old).unwrap();
        fs::write(cc_old.join("session.jsonl"), "{}").unwrap();

        let dst = base.join("new");

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            no_move: false,
            merge: false,
            overwrite: false,
            force: true,
            dry_run: false,
        };

        let result = move_project(&args).unwrap();
        assert!(result.actual_dir_moved);
        assert!(result.cc_dir_renamed);
        assert!(dst.exists());
        assert!(!src.exists());

        let new_san = sanitize_path(&dst.to_string_lossy());
        assert!(projects_dir.join(&new_san).exists());
        assert!(!projects_dir.join(&old_san).exists());

        // Verify session file content survived the move
        let moved_session = projects_dir.join(&new_san).join("session.jsonl");
        assert!(moved_session.exists());
        assert_eq!(fs::read_to_string(moved_session).unwrap(), "{}");
    }

    #[test]
    fn test_move_project_rewrites_history() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let src = base.join("old");
        fs::create_dir(&src).unwrap();
        let dst = base.join("new");

        // Use canonical paths in history entries
        let old_str = src.canonicalize().unwrap().to_string_lossy().to_string();
        let new_str = dst.to_string_lossy().to_string();

        // Create history file
        let history = base.join("history.jsonl");
        let entries = vec![
            format!(r#"{{"project":"{}","sessionId":"abc","timestamp":1}}"#, old_str),
            format!(r#"{{"project":"/other/path","sessionId":"def","timestamp":2}}"#),
            format!(r#"{{"project":"{}","sessionId":"ghi","timestamp":3}}"#, old_str),
        ];
        fs::write(&history, entries.join("\n") + "\n").unwrap();

        // Create projects dir
        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            no_move: false,
            merge: false,
            overwrite: false,
            force: true,
            dry_run: false,
        };

        let result = move_project(&args).unwrap();
        assert_eq!(result.history_lines_updated, 2);

        // Verify history was rewritten
        let content = fs::read_to_string(&history).unwrap();
        assert!(content.contains(&new_str));
        assert!(!content.contains(&old_str));
        // Other entries preserved
        assert!(content.contains("/other/path"));
    }

    #[test]
    fn test_move_project_dry_run() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let src = base.join("old");
        fs::create_dir(&src).unwrap();
        let dst = base.join("new");

        // Create projects dir
        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            no_move: false,
            merge: false,
            overwrite: false,
            force: false,
            dry_run: true,
        };

        let result = move_project(&args).unwrap();
        // Dry run: nothing actually changed
        assert!(!result.actual_dir_moved);
        assert!(!result.cc_dir_renamed);
        // Source still exists
        assert!(src.exists());
        assert!(!dst.exists());
    }

    #[test]
    fn test_clean_orphans_dry_run() {
        let tmp = tempfile::tempdir().unwrap();
        let projects_dir = tmp.path().join("projects");
        fs::create_dir(&projects_dir).unwrap();

        // Create a project whose source doesn't exist (orphan)
        let orphan = projects_dir.join("-nonexistent-path");
        fs::create_dir(&orphan).unwrap();
        fs::write(orphan.join("session.jsonl"), "{}").unwrap();

        let (result, orphans) = clean_orphans(tmp.path(), true).unwrap();
        assert_eq!(result.orphans_found, 1);
        assert_eq!(result.orphans_removed, 0); // dry run
        assert_eq!(orphans.len(), 1);
        // Dir still exists
        assert!(orphan.exists());
    }

    #[test]
    fn test_clean_orphans_removes() {
        let tmp = tempfile::tempdir().unwrap();
        let projects_dir = tmp.path().join("projects");
        fs::create_dir(&projects_dir).unwrap();

        let orphan = projects_dir.join("-nonexistent-path");
        fs::create_dir(&orphan).unwrap();
        fs::write(orphan.join("session.jsonl"), "{}").unwrap();

        let (result, _) = clean_orphans(tmp.path(), false).unwrap();
        assert_eq!(result.orphans_found, 1);
        assert_eq!(result.orphans_removed, 1);
        assert!(!orphan.exists());
    }

    #[test]
    fn test_move_project_already_moved() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        // Only destination exists (user already did `mv`)
        let src = base.join("old");
        let dst = base.join("new");
        fs::create_dir(&dst).unwrap();

        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();
        // src doesn't exist, so use base (already canonical) directly
        let old_san = sanitize_path(&src.to_string_lossy());
        let cc_old = projects_dir.join(&old_san);
        fs::create_dir(&cc_old).unwrap();
        fs::write(cc_old.join("s.jsonl"), "{}").unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            no_move: false,
            merge: false,
            overwrite: false,
            force: false,
            dry_run: false,
        };

        let result = move_project(&args).unwrap();
        assert!(!result.actual_dir_moved); // didn't move dir (already moved)
        assert!(result.cc_dir_renamed); // but renamed CC state
    }

    #[test]
    fn test_move_project_state_only() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();

        let src = base.join("old");
        fs::create_dir(&src).unwrap();
        let dst = base.join("new");
        fs::create_dir(&dst).unwrap();

        let projects_dir = base.join("projects");
        fs::create_dir(&projects_dir).unwrap();
        // Use canonical path for sanitization (matches what resolve_path returns)
        let old_san = sanitize_path(&src.canonicalize().unwrap().to_string_lossy());
        let cc_old = projects_dir.join(&old_san);
        fs::create_dir(&cc_old).unwrap();

        let args = MoveArgs {
            old_path: src.clone(),
            new_path: dst.clone(),
            config_dir: base.clone(),
            no_move: true, // --no-move
            merge: false,
            overwrite: false,
            force: false,
            dry_run: false,
        };

        let result = move_project(&args).unwrap();
        assert!(!result.actual_dir_moved);
        assert!(result.cc_dir_renamed);
        // Both dirs still exist (--no-move)
        assert!(src.exists());
        assert!(dst.exists());
    }
}
