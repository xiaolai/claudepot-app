//! Memory `@include` directive resolver — Rust port of CC's
//! `utils/claudemd.ts` memory-import machinery.
//!
//! CC memory files (`CLAUDE.md`, `.claude/CLAUDE.md`,
//! `.claude/rules/*.md`, `CLAUDE.local.md`, `~/.claude/CLAUDE.md`,
//! `~/.claude/rules/*.md`, `MEMORY.md`) can reference other files with
//! `@path` directives. CC loads each referenced file as additional
//! memory (recursively, up to 5 levels deep) with cycle detection. The
//! Config section must surface every reachable target so users see the
//! full set of bytes the model will receive.
//!
//! Port contract:
//! - Regex, path-shape validation, extension allowlist, fragment
//!   stripping, and escaped-space handling are byte-for-byte parity
//!   with `claudemd.ts:455-535`.
//! - `MAX_DEPTH` matches CC's `MAX_INCLUDE_DEPTH`.
//! - Missing files / non-text extensions / cycles silently skipped
//!   (same as CC).
//! - `includeExternal` gate: User memory always includes external
//!   targets; Project/Local/Managed require the
//!   `hasClaudeMdExternalIncludesApproved` flag from the global
//!   config. The gate is applied by the caller, not this module.

use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// CC's `MAX_INCLUDE_DEPTH` — `claudemd.ts:537`. Include chains deeper
/// than this are truncated.
pub const MAX_DEPTH: usize = 5;

/// Extensions CC treats as text, ported verbatim from
/// `claudemd.ts:96-227`. Non-matching extension → silently skipped.
/// Order preserved so diff against CC reads cleanly.
pub const TEXT_FILE_EXTENSIONS: &[&str] = &[
    // Markdown and text
    ".md", ".txt", ".text",
    // Data formats
    ".json", ".yaml", ".yml", ".toml", ".xml", ".csv",
    // Web
    ".html", ".htm", ".css", ".scss", ".sass", ".less",
    // JavaScript/TypeScript
    ".js", ".ts", ".tsx", ".jsx", ".mjs", ".cjs", ".mts", ".cts",
    // Python
    ".py", ".pyi", ".pyw",
    // Ruby
    ".rb", ".erb", ".rake",
    // Go
    ".go",
    // Rust
    ".rs",
    // Java/Kotlin/Scala
    ".java", ".kt", ".kts", ".scala",
    // C/C++
    ".c", ".cpp", ".cc", ".cxx", ".h", ".hpp", ".hxx",
    // C#
    ".cs",
    // Swift
    ".swift",
    // Shell
    ".sh", ".bash", ".zsh", ".fish", ".ps1", ".bat", ".cmd",
    // Config
    ".env", ".ini", ".cfg", ".conf", ".config", ".properties",
    // Database
    ".sql", ".graphql", ".gql",
    // Protocol
    ".proto",
    // Frontend frameworks
    ".vue", ".svelte", ".astro",
    // Templating
    ".ejs", ".hbs", ".pug", ".jade",
    // Other languages
    ".php", ".pl", ".pm", ".lua", ".r", ".R", ".dart", ".ex", ".exs",
    ".erl", ".hrl", ".clj", ".cljs", ".cljc", ".edn", ".hs", ".lhs",
    ".elm", ".ml", ".mli", ".f", ".f90", ".f95", ".for",
    // Build files
    ".cmake", ".make", ".makefile", ".gradle", ".sbt",
    // Documentation
    ".rst", ".adoc", ".asciidoc", ".org", ".tex", ".latex",
    // Lock files
    ".lock",
    // Misc
    ".log", ".diff", ".patch",
];

/// Returns true when `path`'s extension is in the CC text-file
/// allowlist. No extension → false (CC's `if (ext && …)` at
/// `claudemd.ts:350-354`). Comparison is ASCII-lowercase to match
/// `toLowerCase()` in CC.
pub fn is_text_extension(path: &Path) -> bool {
    let Some(ext) = path.extension() else { return false };
    let ext_lc = ext.to_string_lossy().to_ascii_lowercase();
    let full = format!(".{ext_lc}");
    TEXT_FILE_EXTENSIONS.contains(&full.as_str())
}

/// One resolved include target plus the chain that pulled it in.
#[derive(Clone, Debug)]
pub struct ResolvedInclude {
    pub abs_path: PathBuf,
    pub included_by: PathBuf,
    pub depth: usize,
}

/// Extract `@path` tokens from a memory-file body. Skips fenced code
/// blocks, inline code spans, and non-comment HTML (residue after
/// `<!-- … -->` is re-scanned, matching `claudemd.ts:503-513`).
///
/// Returns absolute `PathBuf`s resolved against `base_dir`. Invalid
/// path shapes and unsupported extensions are dropped here so the
/// caller's recursion budget doesn't spend on dead ends.
pub fn extract_includes(body: &str, base_dir: &Path) -> Vec<PathBuf> {
    let scannable = mask_non_scannable(body);
    let re = include_regex();
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for cap in re.captures_iter(&scannable) {
        let Some(raw) = cap.get(1) else { continue };
        let stripped = strip_fragment(raw.as_str());
        let Some(unescaped) = unescape_spaces(stripped) else {
            continue;
        };
        if !is_valid_path_shape(&unescaped) {
            continue;
        }
        let resolved = expand_path(&unescaped, base_dir);
        if !is_text_extension(&resolved) {
            continue;
        }
        if seen.insert(resolved.clone()) {
            out.push(resolved);
        }
    }
    out
}

/// CC's regex `/(?:^|\s)@((?:[^\s\\]|\\ )+)/g`. The leading alternation
/// ensures the `@` isn't inside another word. Compiled once.
fn include_regex() -> Regex {
    // Rust's `regex` crate supports `(?:…)` but not lookbehind; the
    // leading `(?:^|\s)` is zero-width in the match length sense but
    // consumes one char (the space). That's fine because we only care
    // about the captured group 1.
    Regex::new(r"(?:^|\s)@((?:[^\s\\]|\\ )+)").unwrap()
}

/// Replace fenced-code / inline-code / non-comment HTML spans with
/// equal-length whitespace so offset-sensitive consumers still work.
/// HTML comment residue is preserved in-line; the `<!-- -->` wrapper
/// itself is blanked.
fn mask_non_scannable(body: &str) -> String {
    let bytes = body.as_bytes();
    let mut out = body.to_string();
    // Pass 1: fenced blocks. Look line-by-line for a line whose first
    // non-whitespace run is ``` or ~~~ (3+). Close on the matching
    // marker.
    mask_fenced_blocks(&mut out);
    // Pass 2: inline backticks. Match shortest `…` span.
    mask_inline_code(&mut out);
    // Pass 3: HTML blocks. Comments keep their residue (outside the
    // <!-- --> wrapper); other tags blanked.
    mask_html(&mut out);
    // Sanity: length preserved.
    debug_assert_eq!(out.len(), bytes.len());
    out
}

fn mask_fenced_blocks(text: &mut String) {
    let bytes = unsafe { text.as_bytes_mut() };
    let mut in_fence: Option<(u8, usize)> = None; // (marker, run_len)
    let mut i = 0usize;
    let mut line_start = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            let line = &bytes[line_start..i];
            let trimmed = strip_leading_ws(line);
            let marker = if trimmed.starts_with(b"```") {
                Some(b'`')
            } else if trimmed.starts_with(b"~~~") {
                Some(b'~')
            } else {
                None
            };
            if let Some(m) = marker {
                let run = trimmed.iter().take_while(|&&b| b == m).count();
                match in_fence {
                    None => in_fence = Some((m, run)),
                    Some((open_m, open_run)) if m == open_m && run >= open_run => {
                        // Close fence; blank the fence line too.
                        blank_range(bytes, line_start, i);
                        in_fence = None;
                    }
                    _ => {}
                }
                // When opening the fence also blank the open line.
                if in_fence.is_some() && marker.is_some() {
                    blank_range(bytes, line_start, i);
                }
            } else if in_fence.is_some() {
                blank_range(bytes, line_start, i);
            }
            line_start = i + 1;
        }
        i += 1;
    }
    // Tail line (no trailing newline).
    if in_fence.is_some() {
        blank_range(bytes, line_start, bytes.len());
    }
}

fn mask_inline_code(text: &mut String) {
    let bytes = unsafe { text.as_bytes_mut() };
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            // Count run of backticks to form an opener.
            let open_run = bytes[i..].iter().take_while(|&&b| b == b'`').count();
            let open_end = i + open_run;
            // Find a matching run of same length.
            let mut j = open_end;
            while j + open_run <= bytes.len() {
                if bytes[j..j + open_run].iter().all(|&b| b == b'`') {
                    // Ensure it's not a longer run (would be a different close).
                    let next = j + open_run;
                    if next == bytes.len() || bytes[next] != b'`' {
                        blank_range(bytes, i, next);
                        i = next;
                        break;
                    }
                }
                j += 1;
            }
            if j + open_run > bytes.len() {
                // No matching close — leave as-is.
                i = open_end;
            }
        } else {
            i += 1;
        }
    }
}

fn mask_html(text: &mut String) {
    let bytes = unsafe { text.as_bytes_mut() };
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Comment?
            if bytes[i..].starts_with(b"<!--") {
                // Blank `<!--` and the matching `-->`, keep interior bytes.
                let start = i;
                let open_end = i + 4;
                if let Some(close_rel) = find_subslice(&bytes[open_end..], b"-->") {
                    let close_start = open_end + close_rel;
                    let close_end = close_start + 3;
                    blank_range(bytes, start, open_end);
                    blank_range(bytes, close_start, close_end);
                    i = close_end;
                    continue;
                } else {
                    // Unterminated comment — blank the rest.
                    blank_range(bytes, start, bytes.len());
                    break;
                }
            }
            // Tag? Find matching `>`; blank the whole tag.
            if i + 1 < bytes.len() && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'/' || bytes[i + 1] == b'!') {
                if let Some(close_rel) = find_subslice(&bytes[i..], b">") {
                    let close_end = i + close_rel + 1;
                    blank_range(bytes, i, close_end);
                    i = close_end;
                    continue;
                }
            }
        }
        i += 1;
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    for i in 0..=haystack.len() - needle.len() {
        if &haystack[i..i + needle.len()] == needle {
            return Some(i);
        }
    }
    None
}

fn blank_range(bytes: &mut [u8], start: usize, end: usize) {
    for b in &mut bytes[start..end] {
        if *b != b'\n' {
            *b = b' ';
        }
    }
}

fn strip_leading_ws(line: &[u8]) -> &[u8] {
    let n = line.iter().take_while(|&&b| b == b' ' || b == b'\t').count();
    &line[n..]
}

fn strip_fragment(s: &str) -> &str {
    match s.find('#') {
        Some(i) => &s[..i],
        None => s,
    }
}

fn unescape_spaces(s: &str) -> Option<String> {
    if s.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(s.len());
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        if b == b'\\' {
            if let Some(next) = bytes.next() {
                if next == b' ' {
                    out.push(' ');
                } else {
                    out.push('\\');
                    out.push(next as char);
                }
            } else {
                out.push('\\');
            }
        } else {
            out.push(b as char);
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Path-shape validation mirrored from `claudemd.ts:476-489`.
fn is_valid_path_shape(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    if path == "/" {
        return false;
    }
    if path.starts_with("./") || path.starts_with("~/") {
        return true;
    }
    if path.starts_with('/') {
        return true;
    }
    if path.starts_with('@') {
        return false;
    }
    // Rejected first-char classes from CC's regex `/^[#%^&*()]+/`.
    if let Some(first) = path.chars().next() {
        if matches!(first, '#' | '%' | '^' | '&' | '*' | '(' | ')') {
            return false;
        }
        // Must start with `[a-zA-Z0-9._-]` per
        // `/^[a-zA-Z0-9._-]/.test(path)`.
        if !(first.is_ascii_alphanumeric() || matches!(first, '.' | '_' | '-')) {
            return false;
        }
    }
    true
}

/// Mirror of CC's `utils/path.ts:expandPath`. `~` and `~/…` expand to
/// the user's home dir; `.` / bare paths resolve against `base_dir`;
/// `/…` is absolute.
pub fn expand_path(path: &str, base_dir: &Path) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if path.starts_with('/') {
        return PathBuf::from(path);
    }
    if let Some(rest) = path.strip_prefix("./") {
        return base_dir.join(rest);
    }
    base_dir.join(path)
}

/// Recursively resolve every `@include` reachable from `root_file`.
/// Returns a depth-first walk in CC order (includes emitted *after*
/// the parent — `claudemd.ts:663-684` pushes the main file first,
/// then recurses per include).
///
/// `allow_external_chain` passes through the original `includeExternal`
/// gate. For User memory it's always `true`; Project/Local/Managed
/// callers check `hasClaudeMdExternalIncludesApproved` first. The gate
/// filters targets whose resolved path is outside `original_cwd`.
pub fn resolve_all(
    root_file: &Path,
    original_cwd: &Path,
    allow_external: bool,
) -> Vec<ResolvedInclude> {
    let mut out = Vec::new();
    let mut processed: HashSet<PathBuf> = HashSet::new();
    walk(root_file, original_cwd, allow_external, 0, root_file, &mut processed, &mut out);
    out
}

fn walk(
    file: &Path,
    original_cwd: &Path,
    allow_external: bool,
    depth: usize,
    parent: &Path,
    processed: &mut HashSet<PathBuf>,
    out: &mut Vec<ResolvedInclude>,
) {
    if depth >= MAX_DEPTH {
        return;
    }
    let canon_original = std::fs::canonicalize(file)
        .unwrap_or_else(|_| file.to_path_buf());
    if !processed.insert(canon_original.clone()) {
        return;
    }
    // Also mark the raw path so two different paths to the same logical
    // file don't both recurse (`claudemd.ts:629-648`).
    processed.insert(file.to_path_buf());

    let Ok(bytes) = std::fs::read(file) else { return };
    let Ok(body) = std::str::from_utf8(&bytes) else { return };
    let base_dir = file.parent().unwrap_or(Path::new("."));
    for target in extract_includes(body, base_dir) {
        let canon = std::fs::canonicalize(&target)
            .unwrap_or_else(|_| target.clone());
        if !canon.is_file() {
            continue;
        }
        if !allow_external && !is_inside(&canon, original_cwd) {
            continue;
        }
        if depth + 1 == MAX_DEPTH {
            // Emit leaf without recursing further.
            if processed.insert(canon.clone()) {
                out.push(ResolvedInclude {
                    abs_path: canon.clone(),
                    included_by: parent.to_path_buf(),
                    depth: depth + 1,
                });
            }
            continue;
        }
        if processed.contains(&canon) {
            continue;
        }
        out.push(ResolvedInclude {
            abs_path: canon.clone(),
            included_by: parent.to_path_buf(),
            depth: depth + 1,
        });
        walk(&canon, original_cwd, allow_external, depth + 1, &canon, processed, out);
    }
}

fn is_inside(candidate: &Path, anchor: &Path) -> bool {
    let c = std::fs::canonicalize(candidate).unwrap_or_else(|_| candidate.to_path_buf());
    let a = std::fs::canonicalize(anchor).unwrap_or_else(|_| anchor.to_path_buf());
    c.starts_with(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn extracts_simple_at_path() {
        let td = TempDir::new().unwrap();
        let paths = extract_includes("See @./foo.md for details", td.path());
        assert_eq!(paths, vec![td.path().join("foo.md")]);
    }

    #[test]
    fn extracts_home_relative() {
        let paths = extract_includes("Ref @~/note.md here", Path::new("/x"));
        let home = dirs::home_dir().unwrap();
        assert_eq!(paths, vec![home.join("note.md")]);
    }

    #[test]
    fn extracts_absolute() {
        let paths = extract_includes("Abs @/etc/hosts.md thing", Path::new("/x"));
        assert_eq!(paths, vec![PathBuf::from("/etc/hosts.md")]);
    }

    #[test]
    fn bare_relative_accepted() {
        let td = TempDir::new().unwrap();
        let paths = extract_includes("Foo @notes.md bar", td.path());
        assert_eq!(paths, vec![td.path().join("notes.md")]);
    }

    #[test]
    fn strips_fragment() {
        let td = TempDir::new().unwrap();
        let paths = extract_includes("@./foo.md#section here", td.path());
        assert_eq!(paths, vec![td.path().join("foo.md")]);
    }

    #[test]
    fn unescapes_spaces() {
        let td = TempDir::new().unwrap();
        let paths = extract_includes(r"@./with\ space.md here", td.path());
        assert_eq!(paths, vec![td.path().join("with space.md")]);
    }

    #[test]
    fn skips_fenced_code_block() {
        let td = TempDir::new().unwrap();
        let body = "Before\n```\n@./inside.md should skip\n```\n@./outside.md";
        let paths = extract_includes(body, td.path());
        assert_eq!(paths, vec![td.path().join("outside.md")]);
    }

    #[test]
    fn skips_inline_code() {
        let td = TempDir::new().unwrap();
        let body = "See `@./inside.md` → @./outside.md";
        let paths = extract_includes(body, td.path());
        assert_eq!(paths, vec![td.path().join("outside.md")]);
    }

    #[test]
    fn html_comment_residue_kept() {
        let td = TempDir::new().unwrap();
        // Per claudemd.ts:503-513, the residue of an html comment is
        // re-scanned for @-paths.
        let body = "<!-- note --> @./foo.md ";
        let paths = extract_includes(body, td.path());
        assert_eq!(paths, vec![td.path().join("foo.md")]);
    }

    #[test]
    fn rejects_bad_shape() {
        let td = TempDir::new().unwrap();
        assert!(extract_includes("@#section.md", td.path()).is_empty());
        assert!(extract_includes("@%var.md", td.path()).is_empty());
        assert!(extract_includes("@@double.md", td.path()).is_empty());
    }

    #[test]
    fn rejects_unsupported_extension() {
        let td = TempDir::new().unwrap();
        let paths = extract_includes("@./binary.exe here", td.path());
        assert!(paths.is_empty());
    }

    #[test]
    fn valid_shape_gates() {
        assert!(is_valid_path_shape("./foo.md"));
        assert!(is_valid_path_shape("~/foo.md"));
        assert!(is_valid_path_shape("/foo.md"));
        assert!(is_valid_path_shape("foo.md"));
        assert!(!is_valid_path_shape("/"));
        assert!(!is_valid_path_shape("@foo.md"));
        assert!(!is_valid_path_shape("#foo.md"));
        assert!(!is_valid_path_shape(""));
    }

    #[test]
    fn recursive_depth_cap() {
        let td = TempDir::new().unwrap();
        // Chain 10 files deep; MAX_DEPTH=5 should cap.
        for i in 0..10 {
            let p = td.path().join(format!("{i}.md"));
            let body = if i < 9 {
                format!("hop @./{}.md\n", i + 1)
            } else {
                "leaf".to_string()
            };
            std::fs::write(&p, body).unwrap();
        }
        let root = td.path().join("0.md");
        let out = resolve_all(&root, td.path(), true);
        assert!(out.len() <= MAX_DEPTH);
        assert_eq!(out.first().unwrap().depth, 1);
        // Depth strictly increases along the chain.
        for w in out.windows(2) {
            assert_eq!(w[1].depth, w[0].depth + 1);
        }
    }

    #[test]
    fn cycles_are_broken() {
        let td = TempDir::new().unwrap();
        let a = td.path().join("a.md");
        let b = td.path().join("b.md");
        std::fs::write(&a, "@./b.md").unwrap();
        std::fs::write(&b, "@./a.md").unwrap();
        let out = resolve_all(&a, td.path(), true);
        // Only b is emitted (a is the root and already in processed).
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].abs_path.canonicalize().unwrap(), b.canonicalize().unwrap());
    }

    #[test]
    fn external_gate_drops_outside_cwd() {
        let td = TempDir::new().unwrap();
        let inside_anchor = td.path().join("project");
        std::fs::create_dir(&inside_anchor).unwrap();
        let root = inside_anchor.join("root.md");
        let inside = inside_anchor.join("inside.md");
        let outside = td.path().join("outside.md");
        std::fs::write(&inside, "ok").unwrap();
        std::fs::write(&outside, "ok").unwrap();
        std::fs::write(&root, "@./inside.md and @../outside.md").unwrap();
        let gated = resolve_all(&root, &inside_anchor, false);
        assert_eq!(gated.len(), 1);
        assert_eq!(
            gated[0].abs_path.canonicalize().unwrap(),
            inside.canonicalize().unwrap(),
        );
        let allowed = resolve_all(&root, &inside_anchor, true);
        assert_eq!(allowed.len(), 2);
    }

    #[test]
    fn is_text_extension_matches_common() {
        assert!(is_text_extension(Path::new("/x/a.md")));
        assert!(is_text_extension(Path::new("/x/a.JSON")));
        assert!(is_text_extension(Path::new("/x/a.ts")));
        assert!(!is_text_extension(Path::new("/x/a.png")));
        assert!(!is_text_extension(Path::new("/x/noext")));
    }
}
