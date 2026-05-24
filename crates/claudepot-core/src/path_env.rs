//! Runtime `PATH` enrichment for GUI-launched processes.
//!
//! A macOS app launched from Dock / Finder / Spotlight inherits a
//! minimal `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`) — no Homebrew
//! (`/opt/homebrew/bin`, `/usr/local/bin`), no `~/.local/bin`, no
//! per-user toolchain shims. Any child process Claudepot spawns by
//! *bare name* (`gh`, `brew`, `npm`, an editor) then fails silently:
//! it resolves from a terminal launch and 404s from a Dock launch.
//!
//! [`enriched_path`] **appends** the well-known install directories
//! that actually exist on disk to the inherited `PATH`. Appending —
//! not prepending — is deliberate: the inherited `PATH` always
//! carries the trusted system directories (`/usr/bin`, `/bin`), and
//! keeping them ahead means a user-writable tool directory cannot
//! shadow a system tool for a Claudepot-spawned subprocess. A tool
//! that is genuinely missing from the inherited `PATH` (`gh`,
//! `brew`, a Homebrew editor) still resolves from the appended
//! directories — that is all the Dock-launch fix needs.
//!
//! This is the runtime cousin of
//! [`crate::agent::env::default_path_segments`], which builds
//! a `PATH` *from scratch* with literal `%VAR%` tokens for the
//! scheduler shim files — a different output contract. The two are
//! separate functions on purpose. Their unix user-tool directory
//! sets overlap but are not identical (the shim builder also emits
//! system directories and a Claudepot bin dir); if you add a new
//! install location, check whether both need it.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

/// Well-known directories a CLI tool may live in but that a
/// Dock-launched process will not have on `PATH`. Returned in a
/// stable order — the Anthropic native installer first, Homebrew
/// next (Apple Silicon then Intel/manual), then per-user
/// toolchains. System directories (`/usr/bin`, `/bin`) are omitted
/// — they are already on the minimal inherited `PATH`.
///
/// Directories are returned whether or not they exist on disk;
/// [`enriched_path`] filters to existing ones, while
/// [`crate::fs_utils::find_claude_binary`] joins a filename onto
/// each and probes for the file.
pub fn tool_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let home = dirs::home_dir();
    if let Some(home) = &home {
        dirs.push(home.join(".local/bin"));
    }
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    dirs.push(PathBuf::from("/usr/local/bin"));
    if let Some(home) = &home {
        dirs.push(home.join(".volta/bin"));
        dirs.push(home.join(".bun/bin"));
        dirs.push(home.join(".npm-global/bin"));
    }
    dirs
}

/// Order PATH segments: the inherited entries first, then `extra`
/// appended, with duplicates removed (first occurrence wins).
///
/// Pure — separated from [`enriched_path`] so the ordering and
/// dedup are unit-testable without touching the process
/// environment. Putting `inherited` first is the security-relevant
/// property: see the module docs.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn ordered_segments(inherited: &OsStr, extra: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut segments: Vec<PathBuf> = Vec::new();
    for entry in std::env::split_paths(inherited) {
        // Drop empty components. An empty `PATH` entry means "the
        // current working directory" on Unix — a well-known
        // CWD-on-PATH footgun — and `split_paths("")` yields exactly
        // one such empty entry. Never propagate it.
        if !entry.as_os_str().is_empty() && !segments.contains(&entry) {
            segments.push(entry);
        }
    }
    for dir in extra {
        if !segments.contains(&dir) {
            segments.push(dir);
        }
    }
    segments
}

/// The inherited `PATH` with the existing [`tool_dirs`] appended.
///
/// Only directories that exist on disk are appended, and each is
/// added at most once — an entry already present in the inherited
/// `PATH` is not duplicated. The inherited entries keep their
/// original order and priority; tool directories are added *after*
/// them. On Windows this is a no-op (the Dock-launch problem is
/// macOS/Linux-desktop only) and the inherited `PATH` is returned
/// unchanged.
///
/// Pass the result to `Command::env("PATH", …)` on any builder that
/// spawns a tool by bare name.
pub fn enriched_path() -> OsString {
    let inherited = std::env::var_os("PATH").unwrap_or_default();

    #[cfg(target_os = "windows")]
    {
        inherited
    }

    #[cfg(not(target_os = "windows"))]
    {
        let existing: Vec<PathBuf> = tool_dirs().into_iter().filter(|d| d.is_dir()).collect();
        let segments = ordered_segments(&inherited, existing);
        // join_paths only fails if a segment contains the platform
        // separator — none of ours can — but fall back to the
        // inherited value rather than panicking if it ever does.
        std::env::join_paths(&segments).unwrap_or(inherited)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_dirs_is_priority_ordered() {
        let dirs = tool_dirs();
        let homebrew = dirs
            .iter()
            .position(|d| d == &PathBuf::from("/opt/homebrew/bin"))
            .expect("homebrew dir present");
        let usr_local = dirs
            .iter()
            .position(|d| d == &PathBuf::from("/usr/local/bin"))
            .expect("usr/local/bin present");
        assert!(
            homebrew < usr_local,
            "Apple Silicon Homebrew should rank before Intel/manual"
        );
        // The native installer, when HOME is known, ranks first.
        if dirs::home_dir().is_some() {
            let local = dirs
                .iter()
                .position(|d| d.ends_with(".local/bin"))
                .expect(".local/bin present when HOME is set");
            assert!(local < homebrew, ".local/bin should rank before Homebrew");
        }
    }

    #[test]
    fn test_tool_dirs_omits_system_dirs() {
        // System dirs are already on the inherited minimal PATH;
        // re-listing them would be noise.
        let dirs = tool_dirs();
        assert!(!dirs.iter().any(|d| d == &PathBuf::from("/usr/bin")));
        assert!(!dirs.iter().any(|d| d == &PathBuf::from("/bin")));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_ordered_segments_appends_extra_after_inherited() {
        // The security-relevant property: trusted inherited dirs
        // keep priority; tool dirs land strictly after them.
        //
        // Unix-only because the inherited PATH literal uses `:` as
        // the separator; on Windows `ordered_segments` splits on
        // `;` (via `std::env::split_paths`) and "/usr/bin:/bin" is
        // therefore a single entry. The function logic is identical
        // across platforms; only the test inputs are Unix-shaped.
        let inherited = OsStr::new("/usr/bin:/bin");
        let out = ordered_segments(inherited, vec![PathBuf::from("/opt/homebrew/bin")]);
        assert_eq!(
            out,
            vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
                PathBuf::from("/opt/homebrew/bin"),
            ],
            "tool dirs must be appended after the inherited PATH"
        );
        let usr_bin = out
            .iter()
            .position(|d| d == &PathBuf::from("/usr/bin"))
            .unwrap();
        let brew = out
            .iter()
            .position(|d| d == &PathBuf::from("/opt/homebrew/bin"))
            .unwrap();
        assert!(usr_bin < brew, "/usr/bin must stay ahead of a tool dir");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_ordered_segments_dedups_keeping_first_occurrence() {
        // A tool dir already on the inherited PATH is not re-added,
        // and keeps its original (earlier) position. Unix-only for
        // the same `:` vs `;` reason as
        // `test_ordered_segments_appends_extra_after_inherited`.
        let inherited = OsStr::new("/usr/bin:/opt/homebrew/bin:/bin");
        let out = ordered_segments(
            inherited,
            vec![
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/usr/local/bin"),
            ],
        );
        assert_eq!(
            out,
            vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/bin"),
                PathBuf::from("/usr/local/bin"),
            ],
            "an inherited tool dir keeps its original slot; only the missing one is appended"
        );
    }

    #[test]
    fn test_ordered_segments_empty_inherited() {
        let out = ordered_segments(OsStr::new(""), vec![PathBuf::from("/opt/homebrew/bin")]);
        assert_eq!(out, vec![PathBuf::from("/opt/homebrew/bin")]);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_enriched_path_keeps_inherited_entries_first() {
        // The enriched PATH must START with the inherited entries —
        // deduped and empty-filtered the same way `enriched_path`
        // does — so no appended tool dir can precede a trusted one.
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        let mut expected_prefix: Vec<PathBuf> = Vec::new();
        for e in std::env::split_paths(&inherited) {
            if !e.as_os_str().is_empty() && !expected_prefix.contains(&e) {
                expected_prefix.push(e);
            }
        }
        let enriched = enriched_path();
        let out: Vec<PathBuf> = std::env::split_paths(&enriched).collect();
        for (i, entry) in expected_prefix.iter().enumerate() {
            assert_eq!(
                out.get(i),
                Some(entry),
                "inherited PATH entry {i} must keep its leading position"
            );
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_enriched_path_has_no_duplicate_entries() {
        let enriched = enriched_path();
        let entries: Vec<PathBuf> = std::env::split_paths(&enriched).collect();
        let mut seen = std::collections::HashSet::new();
        for e in &entries {
            assert!(seen.insert(e.clone()), "duplicate PATH entry: {e:?}");
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_enriched_path_omits_nonexistent_dirs() {
        // Production contract: `enriched_path` must not ADD a tool_dir
        // that does not exist on disk. It does NOT promise to scrub
        // nonexistent dirs that are already on the inherited PATH —
        // we trust the user's environment as-is and only refuse to
        // pollute it further. Compute the delta (enriched − inherited)
        // and assert against that, so this test passes on CI runners
        // whose PATH contains e.g. `/opt/homebrew/bin` on Ubuntu.
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        let inherited_entries: Vec<PathBuf> = std::env::split_paths(&inherited).collect();
        let enriched = enriched_path();
        let enriched_entries: Vec<PathBuf> = std::env::split_paths(&enriched).collect();
        let added: Vec<&PathBuf> = enriched_entries
            .iter()
            .filter(|d| !inherited_entries.contains(d))
            .collect();
        for dir in tool_dirs() {
            if !dir.is_dir() {
                assert!(
                    !added.contains(&&dir),
                    "nonexistent dir {dir:?} must not be ADDED to the enriched PATH"
                );
            }
        }
    }
}
