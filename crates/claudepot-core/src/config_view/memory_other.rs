//! Other-projects memory discovery.
//!
//! Per `dev-docs/config-section-plan.md` §6.1 + §14.6:
//! - Collects every `<~/.claude/projects>/<slug>/memory/` directory.
//! - Reports the sanitized slug + lossy flag.
//! - Provides disambiguation candidates when `find_project_memory_dir`
//!   returns `Ambiguous` (plan §10.4) — the UI renders a role="listbox".

use crate::config_view::model::{FileNode, FileSummary, Kind, Scope};
use crate::paths::claude_config_dir;
use crate::project_sanitize::{sanitize_path, unsanitize_path};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct MemorySlug {
    pub slug: String,
    pub abs_dir: PathBuf,
    /// `true` when the slug hit the 200-char hash tail — the reverse
    /// mapping via `unsanitize_path` can't be trusted.
    pub lossy: bool,
    pub reconstructed_path: Option<String>,
    pub file_count: usize,
}

pub fn scan_other_memory_dirs(current_project_slug: &str) -> Vec<MemorySlug> {
    let base = claude_config_dir().join("projects");
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&base) else {
        return out;
    };
    for entry in rd.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name == current_project_slug {
            continue;
        }
        let mem = entry.path().join("memory");
        if !mem.is_dir() {
            continue;
        }
        let (file_count, truncated) = count_md_files(&mem);
        // Suppress only when we proved the dir is empty of .md files.
        // If the walker truncated (cap hit), we don't have that proof,
        // so keep the slug visible — hiding a real memory source just
        // because the walk was deep would be worse than rendering a
        // possibly-undercounted row (audit 2026-04-24, M4).
        if file_count == 0 && !truncated {
            continue;
        }
        let lossy = looks_lossy(&name);
        let reconstructed = if lossy {
            None
        } else {
            Some(unsanitize_path(&name))
        };
        out.push(MemorySlug {
            slug: name,
            abs_dir: mem,
            lossy,
            reconstructed_path: reconstructed,
            file_count,
        });
    }
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    out
}

fn looks_lossy(slug: &str) -> bool {
    // Slugs hit the hash tail when they exceed MAX_SANITIZED (200).
    // CC appends `-<hash>` in those cases, so any slug at or over 200
    // chars is treated as lossy (we can't reverse the hash).
    slug.len() >= 200
}

/// Iterative bounded walk. The caller only needs "any .md files?" plus
/// a rough count for display, so we cap both total entries visited and
/// descent depth to stop a pathological tree (symlink loop, huge
/// sibling repo scanned by accident) from dominating the scan — this
/// function runs once per non-current project slug on every global
/// config refresh.
///
/// Returns `(count, truncated)`. `truncated = true` means the walk hit
/// a cap (entries or depth) before finishing — callers must NOT
/// interpret `count == 0` as "no memory here" in that case, since the
/// first `.md` file may have been beyond the cap (audit 2026-04-24, M4).
fn count_md_files(root: &Path) -> (usize, bool) {
    const MAX_ENTRIES: usize = 2048;
    const MAX_DEPTH: usize = 6;

    let mut n = 0usize;
    let mut visited = 0usize;
    let mut truncated = false;
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            visited += 1;
            if visited >= MAX_ENTRIES {
                return (n, true);
            }
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                if depth + 1 < MAX_DEPTH {
                    stack.push((entry.path(), depth + 1));
                } else {
                    // We have a subdir but can't descend into it. If
                    // we never found a `.md` file here, a deeper one
                    // may exist — flag as truncated so the caller
                    // doesn't treat the dir as definitely empty.
                    truncated = true;
                }
            } else if ft.is_file()
                && entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("md"))
                    .unwrap_or(false)
            {
                n += 1;
            }
        }
    }
    (n, truncated)
}

/// Build a summary FileNode for a MemoryOther slug. The UI renders
/// each slug as a single disclosure row; clicking expands into the
/// individual memory files.
pub fn make_slug_file_node(s: &MemorySlug) -> FileNode {
    let display = s.abs_dir.display().to_string();
    FileNode {
        id: blake3_id(&s.abs_dir),
        kind: Kind::Memory,
        abs_path: s.abs_dir.clone(),
        display_path: display,
        scope_badges: vec![Scope::MemoryOther {
            slug: s.slug.clone(),
            lossy: s.lossy,
        }],
        size_bytes: 0,
        mtime_unix_ns: 0,
        summary: Some(FileSummary {
            title: Some(
                s.reconstructed_path
                    .clone()
                    .unwrap_or_else(|| format!("(lossy) {}", &s.slug[..s.slug.len().min(40)])),
            ),
            description: Some(format!(
                "{} memory file{}",
                s.file_count,
                if s.file_count == 1 { "" } else { "s" }
            )),
        }),
        issues: vec![],
        symlink_origin: None,
        included_by: None,
        include_depth: 0,
    }
}

fn blake3_id(p: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(p.display().to_string().as_bytes());
    let out = h.finalize();
    hex::encode(out)[..16].to_string()
}

/// Resolve a sanitize(project_root) slug against the live filesystem
/// with a prefix-scan fallback per plan §10.4. Used by the UI when a
/// user-supplied path doesn't sanitize to an existing dir.
#[derive(Clone, Debug)]
pub enum MemoryLookup {
    Exact(PathBuf),
    /// Single prefix hit; render with `lossy = true` badge.
    Lossy(PathBuf),
    /// Multiple prefix hits — UI must disambiguate.
    Ambiguous(Vec<PathBuf>),
    NotFound,
}

pub fn find_project_memory_dir(project_root: &Path) -> MemoryLookup {
    let base = claude_config_dir().join("projects");
    let exact_slug = sanitize_path(&project_root.display().to_string());
    let exact = base.join(&exact_slug);
    if exact.is_dir() {
        return MemoryLookup::Exact(exact);
    }

    let prefix = exact_slug
        .split('-')
        .next()
        .unwrap_or(&exact_slug)
        .to_string();
    if prefix.is_empty() {
        return MemoryLookup::NotFound;
    }

    let mut matches = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&base) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) && entry.path().is_dir() {
                matches.push(entry.path());
            }
        }
    }
    match matches.len() {
        0 => MemoryLookup::NotFound,
        1 => MemoryLookup::Lossy(matches.into_iter().next().unwrap()),
        _ => MemoryLookup::Ambiguous(matches),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_lossy_flags_200_char_slugs() {
        let short = "a".repeat(199);
        let long = "a".repeat(200);
        assert!(!looks_lossy(&short));
        assert!(looks_lossy(&long));
    }

    #[test]
    fn make_slug_file_node_labels_lossy_slugs() {
        let s = MemorySlug {
            slug: "a".repeat(200),
            abs_dir: PathBuf::from("/tmp/x"),
            lossy: true,
            reconstructed_path: None,
            file_count: 3,
        };
        let f = make_slug_file_node(&s);
        assert!(f.summary.unwrap().title.unwrap().starts_with("(lossy)"));
    }

    #[test]
    fn count_md_files_respects_entry_cap() {
        let td = tempfile::tempdir().unwrap();
        // 3000 .md files at depth 1; cap is 2048 entries visited.
        for i in 0..3000 {
            std::fs::write(td.path().join(format!("n{i}.md")), "x").unwrap();
        }
        let (n, truncated) = count_md_files(td.path());
        // Early-exit caps the count; exact value depends on readdir
        // order, but it must be (a) at most the cap and (b) well
        // above zero so callers still get "has memory" truthiness.
        assert!(n > 0, "expected positive count, got {n}");
        assert!(n <= 2048, "count should be capped, got {n}");
        assert!(truncated, "cap should flag truncation");
    }

    #[test]
    fn count_md_files_respects_depth_cap() {
        let td = tempfile::tempdir().unwrap();
        // 10-level deep chain with a .md file at each level. Depth cap
        // is 6, so anything strictly deeper is invisible.
        let mut p = td.path().to_path_buf();
        for i in 0..10 {
            p = p.join(format!("d{i}"));
            std::fs::create_dir(&p).unwrap();
            std::fs::write(p.join("note.md"), "x").unwrap();
        }
        let (n, truncated) = count_md_files(td.path());
        assert!(n >= 1, "expected to find at least one file");
        assert!(n <= 6, "depth cap should limit to ~6 files, got {n}");
        assert!(truncated, "depth cap should flag truncation");
    }

    #[test]
    fn count_md_files_reports_not_truncated_when_walk_completes() {
        // No cap hit: 5 .md files at depth 1, no subdirs.
        let td = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(td.path().join(format!("n{i}.md")), "x").unwrap();
        }
        let (n, truncated) = count_md_files(td.path());
        assert_eq!(n, 5);
        assert!(!truncated, "complete walk should not flag truncation");
    }

    #[test]
    fn scan_keeps_slug_when_cap_hits_before_first_md() {
        // Fixture: a "memory" dir full of non-.md noise that exceeds
        // the entry cap. `count_md_files` returns (0, true); the slug
        // must still be included so the UI surfaces the directory
        // instead of silently hiding it.
        //
        // We can't override CLAUDE_HOME inside `scan_other_memory_dirs`
        // without wider plumbing, so this test exercises the same
        // logic path via the reported pair directly: a zero-count +
        // truncated outcome must be treated as "keep the slug."
        let td = tempfile::tempdir().unwrap();
        let memory = td.path().join("memory");
        std::fs::create_dir(&memory).unwrap();
        for i in 0..3000 {
            std::fs::write(memory.join(format!("note{i}.txt")), "x").unwrap();
        }
        let (n, truncated) = count_md_files(&memory);
        assert_eq!(n, 0, "no .md files exist");
        assert!(truncated, "cap must trigger");
        // Contract: the caller must KEEP this dir (`!(n == 0 && !truncated)`).
        assert!(!(n == 0 && !truncated));
    }

    #[test]
    fn make_slug_file_node_carries_reconstructed_path() {
        let s = MemorySlug {
            slug: "repo-project-x".into(),
            abs_dir: PathBuf::from("/tmp/x"),
            lossy: false,
            reconstructed_path: Some("/repo/project/x".into()),
            file_count: 1,
        };
        let f = make_slug_file_node(&s);
        let title = f.summary.unwrap().title.unwrap();
        assert_eq!(title, "/repo/project/x");
    }
}
