//! Other-projects memory discovery.
//!
//! Per `dev-docs/config-section-plan.md` §6.1 + §14.6:
//! - Collects every `<~/.claude/projects>/<slug>/memory/` directory.
//! - Reports the sanitized slug + lossy flag.
//! - Provides disambiguation candidates when `find_project_memory_dir`
//!   returns `Ambiguous` (plan §10.4) — the UI renders a role="listbox".

use crate::config_view::model::{
    FileNode, FileSummary, Kind, Scope,
};
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
    let Ok(rd) = std::fs::read_dir(&base) else { return out };
    for entry in rd.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else { continue };
        if name == current_project_slug {
            continue;
        }
        let mem = entry.path().join("memory");
        if !mem.is_dir() {
            continue;
        }
        let file_count = count_md_files(&mem);
        if file_count == 0 {
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

fn count_md_files(dir: &Path) -> usize {
    let mut n = 0usize;
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            n += count_md_files(&entry.path());
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
    n
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
            description: Some(format!("{} memory file{}", s.file_count, if s.file_count == 1 { "" } else { "s" })),
        }),
        issues: vec![],
        symlink_origin: None,
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

    let prefix = exact_slug.split('-').next().unwrap_or(&exact_slug).to_string();
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
        assert!(f
            .summary
            .unwrap()
            .title
            .unwrap()
            .starts_with("(lossy)"));
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
