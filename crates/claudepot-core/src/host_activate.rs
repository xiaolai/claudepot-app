//! Activate the terminal/editor host of a `claude` process.
//!
//! When a notification fires for a live session, the user wants to
//! return to where work happens — the terminal pane or editor
//! integrated terminal that owns the `claude` process — not to
//! Claudepot's UI. This module walks the process ancestry of a
//! known PID, identifies the topmost macOS app that's a known
//! terminal/editor, and asks LaunchServices to bring it forward.
//!
//! ## Design
//!
//! 1. **Parent walk.** Start at the live session's PID; follow
//!    `parent_pid` upward. Stop on the first ancestor whose
//!    executable path matches a known terminal/editor pattern.
//!    A depth cap (`MAX_DEPTH`) prevents runaway loops if the
//!    process table is malformed.
//!
//! 2. **Identification.** A small hardcoded table of `(exe_match,
//!    bundle_id)` rows. Matching is case-insensitive against the
//!    final component of the executable path. Bundle ids are stable
//!    across versions; the exe name on disk for some apps differs
//!    from the bundle name (VS Code's exe is `Code Helper`, Cursor
//!    spawns through `Cursor Helper`, etc.) so we match against
//!    multiple aliases per app.
//!
//! 3. **Activation.** macOS: `open -b <bundle-id>`. LaunchServices
//!    finds the running instance and brings it to the foreground;
//!    if none is running it launches one (acceptable — the user
//!    asked for that app). No AppleScript permission needed (the
//!    `open` binary is in `/usr/bin`).
//!
//! 4. **Multiplexer caveat.** tmux/zellij/screen run `claude` as a
//!    child of the multiplexer, which itself runs as a child of
//!    the actual terminal. We walk past the multiplexer and land
//!    on the terminal — that's what the user perceives as "where
//!    they were typing." We can't focus the specific pane today;
//!    that's a follow-on per-multiplexer integration.
//!
//! ## Non-macOS
//!
//! Linux/Windows return `Ok(None)` so the renderer falls back to
//! the in-app deep link without an error toast. Adding platform
//! support is a localized change in this module — the call site
//! stays unchanged.

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Hard cap on parent-walk depth. PID 1 is the system init; in
/// practice the chain from `claude` to a terminal is 3-6 hops on
/// macOS (claude → shell → terminal-tab-process → terminal). 24
/// is generous and bounds a malformed-table edge case.
const MAX_DEPTH: usize = 24;

/// Outcome of [`find_host_bundle_id`]. `NotFound` is the natural
/// case for SSH'd remote sessions, daemonized runs, or any unknown
/// host — the caller falls back to in-app routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostLookup {
    /// Resolved to a known macOS bundle id. Pass to [`activate_bundle_id`].
    Found {
        bundle_id: &'static str,
        ancestor_pid: u32,
    },
    /// No known terminal/editor in the ancestry. The caller should
    /// fall back to opening the transcript inside Claudepot.
    NotFound,
    /// The starting PID isn't visible to us (sysinfo couldn't see
    /// it, or it died between the live snapshot and this call).
    /// Treated identically to `NotFound` by the renderer; kept
    /// distinct here for log granularity.
    PidGone,
}

/// Walk parent processes from `pid` upward and return the first
/// ancestor that matches a known terminal/editor.
///
/// Pure read of the process table — no side effects, no IO beyond
/// what `sysinfo::System::refresh_processes` does. Safe to call
/// from any thread.
pub fn find_host_bundle_id(pid: u32) -> HostLookup {
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::new().with_exe(sysinfo::UpdateKind::OnlyIfNotSet),
    );
    find_host_bundle_id_with(&sys, pid)
}

/// Test seam — accepts an externally-prepared `System` so unit tests
/// can drive the parent walk against a synthetic process table.
/// Production callers should use [`find_host_bundle_id`].
pub fn find_host_bundle_id_with(sys: &System, pid: u32) -> HostLookup {
    let mut current = Pid::from_u32(pid);
    if sys.process(current).is_none() {
        return HostLookup::PidGone;
    }
    for _ in 0..MAX_DEPTH {
        let proc_ref = match sys.process(current) {
            Some(p) => p,
            None => return HostLookup::NotFound,
        };
        if let Some(bundle_id) = bundle_id_for_process(proc_ref) {
            return HostLookup::Found {
                bundle_id,
                ancestor_pid: current.as_u32(),
            };
        }
        match proc_ref.parent() {
            Some(parent) if parent != current && parent.as_u32() != 0 => {
                current = parent;
            }
            _ => return HostLookup::NotFound,
        }
    }
    HostLookup::NotFound
}

/// Map a process to a bundle id by inspecting its executable path
/// and process name. We check both: VS Code's renderer process is
/// reported as `Code Helper` (path) but its `name()` may surface a
/// suffix like `Code Helper (Renderer)` on some platforms; iTerm
/// runs as `iTerm2` on disk but ships under `iTerm` in some installs.
fn bundle_id_for_process(p: &sysinfo::Process) -> Option<&'static str> {
    let exe_basename = p
        .exe()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_ascii_lowercase());
    let name_lower = p.name().to_str().map(|s| s.to_ascii_lowercase());

    let candidates: [Option<&str>; 2] = [exe_basename.as_deref(), name_lower.as_deref()];

    for c in candidates.into_iter().flatten() {
        if let Some(b) = match_known_host(c) {
            return Some(b);
        }
    }
    None
}

/// Hardcoded table of known terminal/editor hosts on macOS. Keys
/// match against the lowercase basename of the process exe or its
/// process name — whichever sysinfo surfaces. Multiple aliases per
/// app handle helper-process naming (VS Code, Cursor, Windsurf all
/// run their integrated terminal under `<App> Helper` variants).
///
/// Order is irrelevant — first match wins, but the patterns are
/// disjoint by construction.
fn match_known_host(needle: &str) -> Option<&'static str> {
    // Strip `.app/Contents/MacOS/<exe>` — sysinfo gives the leaf,
    // but be defensive against a future change.
    let leaf = needle
        .rsplit('/')
        .next()
        .unwrap_or(needle)
        .trim_end_matches(".app");

    // Direct, full-name matches (most reliable).
    let direct: &[(&str, &str)] = &[
        ("terminal", "com.apple.Terminal"),
        ("iterm2", "com.googlecode.iterm2"),
        ("iterm", "com.googlecode.iterm2"),
        ("alacritty", "org.alacritty"),
        ("kitty", "net.kovidgoyal.kitty"),
        ("ghostty", "com.mitchellh.ghostty"),
        ("wezterm", "com.github.wez.wezterm"),
        ("wezterm-gui", "com.github.wez.wezterm"),
        ("hyper", "co.zeit.hyper"),
        ("tabby", "org.tabby"),
        ("warp", "dev.warp.Warp-Stable"),
    ];
    for (k, v) in direct {
        if leaf == *k {
            return Some(*v);
        }
    }
    // Helper-process suffix matches. macOS spawns `<App> Helper`
    // children for sandboxed editor extensions; the integrated
    // terminal often lives under one of these. Activating the
    // bundle still correctly raises the parent app's window.
    let prefixed: &[(&str, &str)] = &[
        ("code", "com.microsoft.VSCode"),
        ("cursor", "com.todesktop.230313mzl4w4u92"),
        ("windsurf", "com.exafunction.windsurf"),
    ];
    for (k, v) in prefixed {
        if leaf == *k || leaf.starts_with(&format!("{k} helper")) {
            return Some(*v);
        }
    }
    None
}

/// Activate the macOS app with the given bundle id via
/// LaunchServices (`/usr/bin/open -b <bundle-id>`). Returns Ok(())
/// on a successful spawn — `open` returns immediately and we don't
/// wait on its exit code; the activation is fire-and-forget.
///
/// On non-macOS this is a no-op returning `Ok(())` so the caller
/// doesn't have to branch.
#[allow(clippy::missing_errors_doc)] // os-spawn errors are self-explanatory
pub fn activate_bundle_id(bundle_id: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::process::{Command, Stdio};
        // `-b <bundle-id>`: find by bundle id and bring to front.
        // `-g` would deactivate-self; we omit it so the host app
        // takes focus immediately. Stdin/stdout/stderr discarded —
        // we never read them and don't want to block.
        Command::new("/usr/bin/open")
            .args(["-b", bundle_id])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_child| ())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bundle_id;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_known_host_matches_terminal_aliases() {
        assert_eq!(match_known_host("terminal"), Some("com.apple.Terminal"),);
        assert_eq!(match_known_host("iterm2"), Some("com.googlecode.iterm2"),);
        assert_eq!(match_known_host("iterm"), Some("com.googlecode.iterm2"),);
    }

    #[test]
    fn match_known_host_matches_modern_terminals() {
        assert_eq!(match_known_host("ghostty"), Some("com.mitchellh.ghostty"));
        assert_eq!(match_known_host("wezterm"), Some("com.github.wez.wezterm"));
        assert_eq!(
            match_known_host("wezterm-gui"),
            Some("com.github.wez.wezterm"),
        );
        assert_eq!(match_known_host("alacritty"), Some("org.alacritty"));
        assert_eq!(match_known_host("kitty"), Some("net.kovidgoyal.kitty"));
        assert_eq!(match_known_host("warp"), Some("dev.warp.Warp-Stable"));
    }

    #[test]
    fn match_known_host_handles_helper_processes() {
        // VS Code spawns its integrated terminal under `Code Helper`
        // (and helper subtypes like `Code Helper (Renderer)`).
        // Activating the bundle id raises the main VS Code window
        // regardless of which helper child triggered the lookup.
        assert_eq!(
            match_known_host("code helper"),
            Some("com.microsoft.VSCode"),
        );
        assert_eq!(
            match_known_host("code helper (renderer)"),
            Some("com.microsoft.VSCode"),
        );
        assert_eq!(match_known_host("code"), Some("com.microsoft.VSCode"));
        assert_eq!(
            match_known_host("cursor helper"),
            Some("com.todesktop.230313mzl4w4u92"),
        );
        assert_eq!(
            match_known_host("windsurf helper"),
            Some("com.exafunction.windsurf"),
        );
    }

    #[test]
    fn match_known_host_strips_app_suffix() {
        // Defensive: if a future sysinfo version surfaces the .app
        // form, we still match.
        assert_eq!(
            match_known_host("Terminal.app"),
            None, // case sensitivity preserved — caller lowercases
        );
        assert_eq!(match_known_host("terminal.app"), Some("com.apple.Terminal"),);
    }

    #[test]
    fn match_known_host_rejects_unknown_processes() {
        assert_eq!(match_known_host("zsh"), None);
        assert_eq!(match_known_host("bash"), None);
        assert_eq!(match_known_host("tmux"), None);
        assert_eq!(match_known_host("node"), None);
        assert_eq!(match_known_host("claude"), None);
        // A process whose name *contains* a known token but isn't
        // one — we only match exact tokens or known helper prefixes.
        assert_eq!(match_known_host("warpdrive-server"), None);
    }

    /// Live System call: a process tree must terminate at PID 1
    /// without infinite-looping. This is the only test that reads
    /// the real process table; it asserts the depth cap holds.
    #[test]
    fn parent_walk_terminates() {
        // Walk from this test process upward. We don't assert any
        // particular result — this test process's host could be
        // `cargo test`'s parent shell, an IDE's terminal, or a CI
        // agent. We only assert the walk DOES terminate (no panic,
        // no infinite loop).
        let pid = std::process::id();
        let _ = find_host_bundle_id(pid);
    }
}
