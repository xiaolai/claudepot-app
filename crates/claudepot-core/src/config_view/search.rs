//! Content search engine for the Config section.
//!
//! Plain substring or compiled regex over **already-masked** file
//! bodies. Hits stream through a caller-provided `on_hit` callback so
//! the UI can render them as they arrive. Caps: 200 total, 20 per file,
//! 2 MB per file, per plan §12.6.
//!
//! Thread-safety: the engine is synchronous but designed to be hosted
//! inside a tokio `spawn_blocking` task. Callers pass `CancelToken` to
//! abort mid-stream.

use crate::config_view::mask::mask_bytes;
use crate::config_view::model::{ConfigTree, FileNode, Kind, Node};
use regex::Regex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const GLOBAL_HIT_CAP: u32 = 200;
pub const PER_FILE_HIT_CAP: u32 = 20;
pub const MAX_BODY_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct SearchQuery {
    pub text: String,
    pub regex: bool,
    pub case_sensitive: bool,
    /// Restrict to these scope ids. Empty / `None` = all scopes.
    pub scope_filter: Option<Vec<String>>,
    pub kind_filter: Option<Vec<Kind>>,
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub node_id: String,
    pub line_number: u32,
    pub snippet: String,
    pub match_count_in_file: u32,
}

#[derive(Clone, Debug)]
pub struct SearchSummary {
    pub total_hits: u32,
    pub capped: bool,
    pub skipped_large: u32,
    pub cancelled: bool,
}

/// Cancellation token — flip it to halt mid-scan. Safe to share across
/// threads via `Arc`.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    inner: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.inner.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.inner.load(Ordering::Relaxed)
    }
}

/// Input for one round of matching — either a regex or a plain substring.
enum Matcher {
    Plain { needle: String, case_sensitive: bool },
    Regex(Regex),
}

impl Matcher {
    fn build(query: &SearchQuery) -> Result<Self, String> {
        if query.regex {
            let pat = if query.case_sensitive {
                query.text.clone()
            } else {
                format!("(?i){}", query.text)
            };
            Regex::new(&pat).map(Matcher::Regex).map_err(|e| e.to_string())
        } else {
            Ok(Matcher::Plain {
                needle: query.text.clone(),
                case_sensitive: query.case_sensitive,
            })
        }
    }

    fn find_iter<'a>(&'a self, hay: &'a str) -> Box<dyn Iterator<Item = (usize, usize)> + 'a> {
        match self {
            Matcher::Plain { needle, case_sensitive } => {
                if *case_sensitive {
                    let n = needle.clone();
                    Box::new(plain_iter(hay, n))
                } else {
                    Box::new(icase_plain_iter(hay, needle.to_lowercase()))
                }
            }
            Matcher::Regex(re) => Box::new(re.find_iter(hay).map(|m| (m.start(), m.end()))),
        }
    }
}

fn plain_iter(hay: &str, needle: String) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut i = 0usize;
    std::iter::from_fn(move || {
        if needle.is_empty() || i >= hay.len() {
            return None;
        }
        match hay[i..].find(&needle) {
            Some(off) => {
                let start = i + off;
                let end = start + needle.len();
                i = end;
                Some((start, end))
            }
            None => {
                i = hay.len();
                None
            }
        }
    })
}

fn icase_plain_iter(hay: &str, needle_lc: String) -> impl Iterator<Item = (usize, usize)> + '_ {
    let lc = hay.to_lowercase();
    let mut i = 0usize;
    std::iter::from_fn(move || {
        if needle_lc.is_empty() || i >= lc.len() {
            return None;
        }
        match lc[i..].find(&needle_lc) {
            Some(off) => {
                let start = i + off;
                let end = start + needle_lc.len();
                i = end;
                // Map back to original string byte offsets — works because
                // ASCII-case folding preserves byte offsets, and non-ASCII
                // preserved codepoint boundaries.
                Some((start, end))
            }
            None => {
                i = lc.len();
                None
            }
        }
    })
}

/// Walk the tree, scan each matching file, emit hits via `on_hit` as
/// they're discovered. Returns the summary once finished or cancelled.
pub fn search<F: FnMut(SearchHit)>(
    tree: &ConfigTree,
    query: SearchQuery,
    cancel: &CancelToken,
    mut on_hit: F,
) -> Result<SearchSummary, String> {
    let matcher = Matcher::build(&query)?;

    let scope_filter: Option<std::collections::HashSet<&str>> =
        query.scope_filter.as_ref().map(|v| v.iter().map(String::as_str).collect());
    let kind_filter: Option<std::collections::HashSet<&Kind>> =
        query.kind_filter.as_ref().map(|v| v.iter().collect());

    let mut total: u32 = 0;
    let mut capped = false;
    let mut skipped = 0u32;

    for scope in &tree.scopes {
        if cancel.is_cancelled() {
            return Ok(SearchSummary {
                total_hits: total,
                capped,
                skipped_large: skipped,
                cancelled: true,
            });
        }
        if let Some(sf) = &scope_filter {
            if !sf.contains(scope.id.as_str()) {
                continue;
            }
        }
        for f in collect_files(&scope.children) {
            if cancel.is_cancelled() {
                return Ok(SearchSummary {
                    total_hits: total,
                    capped,
                    skipped_large: skipped,
                    cancelled: true,
                });
            }
            if let Some(kf) = &kind_filter {
                if !kf.contains(&f.kind) {
                    continue;
                }
            }
            // Skip effective/plugin-base virtual + redacted user config.
            if matches!(
                f.kind,
                Kind::EffectiveSettings | Kind::EffectiveMcp | Kind::RedactedUserConfig
            ) {
                continue;
            }

            if f.size_bytes > MAX_BODY_BYTES {
                skipped += 1;
                continue;
            }

            let Ok(bytes) = std::fs::read(&f.abs_path) else {
                continue;
            };
            let body = mask_bytes(&bytes);

            let (file_count, file_capped) = scan_file(&matcher, f.id.clone(), &body, &mut on_hit);
            total += file_count;
            if total >= GLOBAL_HIT_CAP {
                capped = true;
                break;
            }
            if file_capped {
                // per-file cap signalled but we continue scanning other files.
            }
        }
        if capped {
            break;
        }
    }

    Ok(SearchSummary {
        total_hits: total.min(GLOBAL_HIT_CAP),
        capped,
        skipped_large: skipped,
        cancelled: cancel.is_cancelled(),
    })
}

fn collect_files(nodes: &[Node]) -> Vec<&FileNode> {
    let mut out = Vec::new();
    for n in nodes {
        match n {
            Node::File(f) => out.push(f),
            Node::Dir(d) => out.extend(collect_files(&d.children)),
        }
    }
    out
}

/// Scan a single file's masked body. Emits up to `PER_FILE_HIT_CAP`
/// hits. Returns `(total_in_file, file_capped)`.
fn scan_file<F: FnMut(SearchHit)>(
    matcher: &Matcher,
    node_id: String,
    body: &str,
    on_hit: &mut F,
) -> (u32, bool) {
    // Precompute line starts for O(hits) line-number lookup.
    let mut line_starts: Vec<usize> = vec![0];
    for (i, b) in body.as_bytes().iter().enumerate() {
        if *b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    let find_line = |off: usize| -> u32 {
        match line_starts.binary_search(&off) {
            Ok(i) => (i + 1) as u32,
            Err(i) => i as u32, // i is the line number (1-indexed, because vec starts with 0)
        }
    };

    let mut count = 0u32;
    let mut capped = false;
    for (start, _end) in matcher.find_iter(body) {
        if count >= PER_FILE_HIT_CAP {
            capped = true;
            break;
        }
        count += 1;
        let line_no = find_line(start);
        let snippet = snippet_around(body, &line_starts, line_no, 2);
        on_hit(SearchHit {
            node_id: node_id.clone(),
            line_number: line_no,
            snippet,
            match_count_in_file: count,
        });
    }
    (count, capped)
}

fn snippet_around(body: &str, line_starts: &[usize], line_no: u32, context: u32) -> String {
    let total_lines = line_starts.len() as u32;
    let from = line_no.saturating_sub(context).max(1);
    let to = (line_no + context).min(total_lines);
    let begin = line_starts[(from - 1) as usize];
    let end = if (to as usize) < line_starts.len() {
        line_starts[to as usize].saturating_sub(1)
    } else {
        body.len()
    };
    body[begin..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_view::model::{FileNode, Scope, ScopeNode};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn make_tree(files: Vec<(PathBuf, String)>) -> ConfigTree {
        let children: Vec<Node> = files
            .iter()
            .map(|(p, _)| Node::File(FileNode {
                id: format!("id-{}", p.display()),
                kind: Kind::ClaudeMd,
                abs_path: p.clone(),
                display_path: p.display().to_string(),
                scope_badges: vec![Scope::User],
                size_bytes: std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
                mtime_unix_ns: 0,
                summary: None,
                issues: vec![],
                symlink_origin: None,
            }))
            .collect();

        ConfigTree {
            scopes: vec![ScopeNode {
                id: "scope:user".into(),
                scope: Scope::User,
                label: "User".into(),
                recursive_count: children.len(),
                children,
            }],
            scanned_at_unix_ns: 0,
            cwd: PathBuf::from("/"),
            project_root: PathBuf::from("/"),
            memory_slug: "".into(),
            memory_slug_lossy: false,
            cc_version_hint: None,
            enterprise_mcp_lockout: false,
        }
    }

    #[test]
    fn finds_plain_substring_hits() {
        let td = TempDir::new().unwrap();
        let f = td.path().join("a.md");
        std::fs::write(&f, "alpha\nbeta hit gamma\nhit again\n").unwrap();
        let tree = make_tree(vec![(f, String::new())]);

        let mut hits = Vec::new();
        let s = search(
            &tree,
            SearchQuery {
                text: "hit".into(),
                regex: false,
                case_sensitive: true,
                scope_filter: None,
                kind_filter: None,
            },
            &CancelToken::new(),
            |h| hits.push(h),
        )
        .unwrap();
        assert_eq!(s.total_hits, 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].line_number, 2);
        assert_eq!(hits[1].line_number, 3);
        assert!(!s.cancelled);
    }

    #[test]
    fn regex_mode_matches() {
        let td = TempDir::new().unwrap();
        let f = td.path().join("r.md");
        std::fs::write(&f, "foo123 bar456\n").unwrap();
        let tree = make_tree(vec![(f, String::new())]);

        let mut hits = Vec::new();
        let _ = search(
            &tree,
            SearchQuery {
                text: r"\b\w+\d+\b".into(),
                regex: true,
                case_sensitive: false,
                scope_filter: None,
                kind_filter: None,
            },
            &CancelToken::new(),
            |h| hits.push(h),
        )
        .unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn case_insensitive_matches() {
        let td = TempDir::new().unwrap();
        let f = td.path().join("c.md");
        std::fs::write(&f, "HELLO\nworld hello\n").unwrap();
        let tree = make_tree(vec![(f, String::new())]);

        let mut hits = Vec::new();
        let _ = search(
            &tree,
            SearchQuery {
                text: "hello".into(),
                regex: false,
                case_sensitive: false,
                scope_filter: None,
                kind_filter: None,
            },
            &CancelToken::new(),
            |h| hits.push(h),
        )
        .unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn cancellation_stops_mid_scan() {
        let td = TempDir::new().unwrap();
        let mut files = Vec::new();
        for i in 0..5 {
            let p = td.path().join(format!("{i}.md"));
            std::fs::write(&p, "needle\nneedle\nneedle\n").unwrap();
            files.push((p, String::new()));
        }
        let tree = make_tree(files);
        let cancel = CancelToken::new();
        cancel.cancel(); // cancel before we start

        let mut hits = Vec::new();
        let s = search(
            &tree,
            SearchQuery {
                text: "needle".into(),
                regex: false,
                case_sensitive: true,
                scope_filter: None,
                kind_filter: None,
            },
            &cancel,
            |h| hits.push(h),
        )
        .unwrap();
        assert!(s.cancelled);
        assert_eq!(hits.len(), 0);
    }

    #[test]
    fn skips_files_over_2mb() {
        // Construct an oversized file marker — we can't easily write 2 MB
        // in a unit test, so craft the tree directly with a fake size.
        let td = TempDir::new().unwrap();
        let f = td.path().join("big.md");
        std::fs::write(&f, "x").unwrap();
        let mut tree = make_tree(vec![(f.clone(), String::new())]);
        // Force the file node's recorded size over the cap — mimics
        // what discover.rs would report for a huge file.
        if let Node::File(ref mut fnode) = tree.scopes[0].children[0] {
            fnode.size_bytes = MAX_BODY_BYTES + 1;
        }

        let s = search(
            &tree,
            SearchQuery {
                text: "x".into(),
                regex: false,
                case_sensitive: true,
                scope_filter: None,
                kind_filter: None,
            },
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();
        assert_eq!(s.skipped_large, 1);
        assert_eq!(s.total_hits, 0);
    }

    #[test]
    fn snippet_contains_match_line() {
        let td = TempDir::new().unwrap();
        let f = td.path().join("s.md");
        std::fs::write(&f, "a\nb hit c\nd\n").unwrap();
        let tree = make_tree(vec![(f, String::new())]);
        let mut hits = Vec::new();
        let _ = search(
            &tree,
            SearchQuery {
                text: "hit".into(),
                regex: false,
                case_sensitive: true,
                scope_filter: None,
                kind_filter: None,
            },
            &CancelToken::new(),
            |h| hits.push(h),
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("hit"));
    }

    #[test]
    fn search_never_echoes_raw_secret() {
        // Put a known secret in the body; the mask pipeline must hide it.
        let tok = format!("ghp_{}", "A".repeat(40));
        let td = TempDir::new().unwrap();
        let f = td.path().join("s.md");
        std::fs::write(&f, format!("token is {tok} here\n")).unwrap();
        let tree = make_tree(vec![(f, String::new())]);
        let mut hits = Vec::new();
        let _ = search(
            &tree,
            SearchQuery {
                text: "token".into(),
                regex: false,
                case_sensitive: true,
                scope_filter: None,
                kind_filter: None,
            },
            &CancelToken::new(),
            |h| hits.push(h),
        )
        .unwrap();
        assert!(hits.iter().all(|h| !h.snippet.contains(&tok)));
    }
}
