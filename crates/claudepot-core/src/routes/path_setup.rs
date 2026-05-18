//! Shell-PATH integration for CLI wrappers.
//!
//! Claudepot materializes route wrappers under `~/.claudepot/bin/`
//! (see [`super::wrapper`]). That directory is inert until it's on
//! the user's interactive-shell PATH — without it, `routes_use_cli`
//! writes a binary the terminal can't resolve. This module answers
//! the two questions the Third-party UI needs:
//!
//!   - **Is the wrapper dir on PATH?** — [`wrapper_dir_path_status`]
//!     spawns the user's login shell and inspects *its* `$PATH`. A
//!     GUI app's own `$PATH` is not a usable proxy: on macOS an app
//!     launched from Finder/Dock inherits a minimal `launchd` env
//!     that never sourced the user's shell rc. Asking the shell is
//!     the only honest answer.
//!   - **Put it on PATH.** — [`add_wrapper_dir_to_path`] appends an
//!     `export` line to the shell rc file the interactive shell
//!     actually sources, idempotently.

use std::path::{Path, PathBuf};

use super::error::RouteError;
use super::wrapper::wrapper_dir;

/// Comment line stamped above the appended `export`, so a human
/// reading the rc file can recognize where the line came from. It
/// is *not* used for the idempotency decision — see
/// [`body_has_wrapper_dir`].
const RC_MARKER: &str = "# Added by Claudepot — third-party route wrappers on PATH";

/// `\037` (US, unit separator) brackets the probe's PATH output so
/// an interactive rc file that prints a banner to stdout can't be
/// mistaken for the PATH value.
const PROBE_CMD: &str = r#"printf '\037%s\037' "$PATH""#;
const PROBE_DELIM: char = '\u{1f}';

/// Whether `~/.claudepot/bin` is reachable from a terminal the user
/// opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStatus {
    /// The wrapper dir is on the interactive shell's PATH.
    OnPath,
    /// The wrapper dir is not on PATH — wrappers won't resolve.
    NotOnPath,
    /// Couldn't determine: the probe failed or timed out, or the
    /// platform has no POSIX shell. The UI must not claim "on PATH"
    /// in this state.
    Unknown,
}

impl PathStatus {
    /// Stable wire string for the IPC boundary.
    pub fn as_str(&self) -> &'static str {
        match self {
            PathStatus::OnPath => "on_path",
            PathStatus::NotOnPath => "not_on_path",
            PathStatus::Unknown => "unknown",
        }
    }
}

/// Pull the PATH value back out of the sentinel-bracketed probe
/// output. Returns `None` if the delimiters aren't both present.
fn extract_probe_path(stdout: &str) -> Option<&str> {
    let start = stdout.find(PROBE_DELIM)? + PROBE_DELIM.len_utf8();
    let rest = &stdout[start..];
    let end = rest.find(PROBE_DELIM)?;
    Some(&rest[..end])
}

/// Is `dir` one of the `:`-separated entries in a `PATH` string?
fn path_var_contains(path_var: &str, dir: &Path) -> bool {
    path_var
        .split(':')
        .filter(|e| !e.is_empty())
        .any(|entry| Path::new(entry) == dir)
}

/// The shell that drives PATH operations. Honors `$SHELL`; when it's
/// unset or empty — possible in a sparse GUI-launch environment —
/// falls back to the platform's default login shell so PATH setup
/// stays a best effort instead of degrading to "unsupported".
#[cfg(unix)]
fn resolve_shell() -> std::ffi::OsString {
    if let Some(s) = std::env::var_os("SHELL") {
        if !s.is_empty() {
            return s;
        }
    }
    // macOS has shipped zsh as the default login shell since Catalina;
    // elsewhere POSIX sh is the safe floor.
    if cfg!(target_os = "macos") {
        std::ffi::OsString::from("/bin/zsh")
    } else {
        std::ffi::OsString::from("/bin/sh")
    }
}

/// Pick the rc file the interactive shell sources. `None` for shells
/// whose config syntax we don't auto-edit (e.g. fish — `set -gx` /
/// `fish_add_path`, a different grammar). The caller turns `None`
/// into a "edit it by hand" error so we never write a broken line.
fn rc_path_for_shell(shell: &str, home: &Path) -> Option<PathBuf> {
    // `$SHELL` is an absolute path (`/bin/zsh`); match on the binary.
    let name = Path::new(shell).file_name().and_then(|n| n.to_str())?;
    match name {
        "zsh" => Some(home.join(".zshrc")),
        // macOS Terminal launches bash as a *login* shell, which
        // sources `.bash_profile`, not `.bashrc`.
        "bash" => Some(home.join(".bash_profile")),
        _ => None,
    }
}

/// Equivalent PATH-entry spellings for `dir`. When `dir` sits under
/// `home`, a hand-written rc line could spell it absolutely, as
/// `$HOME/…`, or as `~/…` — all three must be recognized by the
/// idempotency check, and we emit the `$HOME/…` form ourselves. The
/// preferred (emitted) form is always first.
fn path_entry_forms(dir: &Path, home: &Path) -> Vec<String> {
    let abs = dir.display().to_string();
    match dir.strip_prefix(home) {
        Ok(rel) => {
            let rel = rel.display();
            vec![format!("$HOME/{rel}"), format!("~/{rel}"), abs]
        }
        Err(_) => vec![abs],
    }
}

/// Whether `dir` is safe to interpolate into a shell rc `export`
/// line. The rc file is `source`d, so a path containing `"`, `'`,
/// `$`, a backtick, or a backslash could corrupt the line or run
/// shell expansion. `$` is rejected even though the emitted line
/// uses `$HOME`/`$PATH` — those tokens are added by us, never
/// sourced from `dir`. Non-UTF-8 paths are refused outright.
fn path_is_shell_safe(dir: &Path) -> bool {
    match dir.to_str() {
        Some(s) => !s
            .chars()
            .any(|c| matches!(c, '"' | '\'' | '`' | '$' | '\\') || c.is_control()),
        None => false,
    }
}

/// The block appended to the rc file: a marker comment + the export.
/// Emits the `$HOME/…` form when possible so the rc line stays
/// portable across machines. The caller must have already gated
/// `dir` through [`path_is_shell_safe`].
fn path_export_block(dir: &Path, home: &Path) -> String {
    let entry = path_entry_forms(dir, home)
        .into_iter()
        .next()
        .expect("path_entry_forms is never empty");
    format!("\n{RC_MARKER}\nexport PATH=\"{entry}:$PATH\"\n")
}

/// True when `entry` appears in `body` as a *delimited* PATH entry —
/// bounded by a shell delimiter (`:`, quote, `=`, whitespace) or the
/// string edge on both sides. A raw substring check would wrongly
/// treat `…/.claudepot/bin` as present inside `…/.claudepot/bin-old`
/// or `…/.claudepot/bin/sub`.
fn contains_path_entry(body: &str, entry: &str) -> bool {
    if entry.is_empty() {
        return false;
    }
    let bytes = body.as_bytes();
    // `(` / `)` are boundaries too, so the zsh array form
    // `path=($HOME/.claudepot/bin $path)` is recognized.
    let is_boundary = |b: u8| {
        matches!(
            b,
            b':' | b'"' | b'\'' | b'=' | b'(' | b')' | b' ' | b'\t' | b'\r' | b'\n'
        )
    };
    let mut search_from = 0;
    while let Some(rel) = body[search_from..].find(entry) {
        let start = search_from + rel;
        let end = start + entry.len();
        let before_ok = start == 0 || is_boundary(bytes[start - 1]);
        let after_ok = end == bytes.len() || is_boundary(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        search_from = start + 1;
    }
    false
}

/// Does this rc line *begin a PATH assignment*? Models the shell
/// assignment-statement shape: optional leading whitespace, an
/// optional declaration keyword (`export` / `typeset` / `declare` /
/// `local` / `readonly`) with its flags, then `PATH=` / `PATH+=` /
/// (zsh) `path=(`. Comment lines, and lines that merely *mention*
/// `PATH=` inside a command or string (`echo "PATH=…"`), are
/// rejected — they don't *start* an assignment.
///
/// This is a deliberately shallow shell-syntax model, not a parser:
/// the idempotency check it backs is best-effort, and the cost of a
/// miss is at worst a duplicate `export` line, never breakage.
fn is_path_assignment(line: &str) -> bool {
    let mut t = line.trim_start();
    if t.starts_with('#') {
        return false;
    }
    for kw in ["export ", "typeset ", "declare ", "local ", "readonly "] {
        if let Some(rest) = t.strip_prefix(kw) {
            t = rest.trim_start();
            // Skip flag words (`-x`, `-gx`, …) between the keyword
            // and the variable name.
            while t.starts_with('-') {
                t = match t.split_once(char::is_whitespace) {
                    Some((_, rest)) => rest.trim_start(),
                    None => "",
                };
            }
            break;
        }
    }
    t.starts_with("PATH=") || t.starts_with("PATH+=") || t.starts_with("path=(")
}

/// The *value* portion of a PATH-assignment line: everything after
/// the first `=` (covers `PATH=`, `PATH+=`, `export PATH=`, zsh
/// `path=(`), with a trailing inline comment (` #…`) or `;`-separated
/// command stripped. The caller has already confirmed the line via
/// [`is_path_assignment`]. Scanning the value — not the whole line —
/// keeps a path mentioned in a trailing comment or command from
/// being mistaken for an actual PATH entry.
///
/// Best-effort: a `#` or `;` *inside* a quoted value would need a
/// real shell parser to handle, and is not a real-world rc shape.
fn path_assignment_value(line: &str) -> &str {
    let t = line.trim_start();
    let mut value = match t.find('=') {
        Some(i) => &t[i + 1..],
        None => return "",
    };
    if let Some(i) = value.find(" #") {
        value = &value[..i];
    }
    if let Some(i) = value.find(';') {
        value = &value[..i];
    }
    value
}

/// True when the rc file already puts the wrapper dir on PATH, in
/// any of its spellings (absolute, `$HOME/…`, `~/…`). The decision
/// is scoped to the *value* of an actual PATH-assignment line — not
/// [`RC_MARKER`], not a mention in a comment or unrelated variable,
/// and not a trailing inline comment or command on the assignment
/// line — so nothing incidental can suppress a real append.
fn body_has_wrapper_dir(body: &str, dir: &Path, home: &Path) -> bool {
    let forms = path_entry_forms(dir, home);
    body.lines()
        .filter(|line| is_path_assignment(line))
        .map(path_assignment_value)
        .any(|value| forms.iter().any(|form| contains_path_entry(value, form)))
}

/// Probe the user's login+interactive shell for whether the wrapper
/// dir is on its PATH. Never errors and never hangs — an
/// indeterminate environment, a spawn failure, or a slow rc file all
/// collapse to [`PathStatus::Unknown`] so callers have one fewer
/// failure mode to thread through.
#[cfg(unix)]
pub async fn wrapper_dir_path_status() -> PathStatus {
    use tokio::time::{timeout, Duration};

    // A login+interactive shell can wedge on a slow rc file (network
    // setup, an interactive prompt). Cap the probe so it can never
    // hang the Tauri command that awaits it.
    const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

    let shell = resolve_shell();
    // `-ilc`: interactive + login, so both zsh (`.zshrc`) and bash
    // (`.bash_profile`) source the file the user edits.
    // `kill_on_drop`: if the timeout fires, the child is reaped with
    // the dropped future instead of leaking.
    let mut cmd = tokio::process::Command::new(&shell);
    cmd.arg("-ilc").arg(PROBE_CMD).kill_on_drop(true);

    let output = match timeout(PROBE_TIMEOUT, cmd.output()).await {
        Ok(Ok(output)) => output,
        // Spawn error or timeout — either way, indeterminate.
        Ok(Err(_)) | Err(_) => return PathStatus::Unknown,
    };
    if !output.status.success() {
        return PathStatus::Unknown;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    match extract_probe_path(&stdout) {
        Some(path_var) if path_var_contains(path_var, &wrapper_dir()) => PathStatus::OnPath,
        Some(_) => PathStatus::NotOnPath,
        None => PathStatus::Unknown,
    }
}

#[cfg(not(unix))]
pub async fn wrapper_dir_path_status() -> PathStatus {
    PathStatus::Unknown
}

/// Append the wrapper-dir `export` line to the user's shell rc file.
/// Idempotent: if the rc already references the dir, this is a no-op
/// and still returns `Ok` with the rc path. Returns the path of the
/// file written so the caller can name it in a confirmation toast.
#[cfg(unix)]
pub fn add_wrapper_dir_to_path() -> Result<PathBuf, RouteError> {
    use std::io::Write;

    // Serialize read-check-append within the process so two rapid
    // "Add to PATH" invocations can't both observe "absent" and
    // append duplicate blocks. Cross-process races aren't a concern
    // — Claudepot is a single-instance desktop app. The guarded data
    // is `()`, so a poisoned lock carries no corruption; recover it.
    static RC_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = RC_WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let home = dirs::home_dir().ok_or(RouteError::NoHomeDir)?;
    let dir = wrapper_dir();

    // Zero-trust at the boundary: the rc file is `source`d, so refuse
    // to write a path that could corrupt the line or inject shell
    // expansion. The normal `$HOME/.claudepot/bin` is always clean;
    // only an exotic `CLAUDEPOT_DATA_DIR` override can trip this.
    if !path_is_shell_safe(&dir) {
        return Err(RouteError::UnsupportedPlatform(
            "wrapper directory path contains characters unsafe for a shell \
             rc file — point CLAUDEPOT_DATA_DIR at a path without quotes, \
             '$', backticks, or backslashes",
        ));
    }

    let shell = resolve_shell();
    let shell = shell.to_string_lossy();
    let rc = rc_path_for_shell(&shell, &home).ok_or(RouteError::UnsupportedPlatform(
        "automatic PATH setup supports zsh and bash — add the wrapper \
         directory to your shell config by hand",
    ))?;

    // Idempotency: never append twice. A missing rc file just means
    // "not present", so it falls through to the append (which
    // creates it).
    if let Ok(body) = std::fs::read_to_string(&rc) {
        if body_has_wrapper_dir(&body, &dir, &home) {
            return Ok(rc);
        }
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&rc)?;
    file.write_all(path_export_block(&dir, &home).as_bytes())?;
    Ok(rc)
}

#[cfg(not(unix))]
pub fn add_wrapper_dir_to_path() -> Result<PathBuf, RouteError> {
    Err(RouteError::UnsupportedPlatform(
        "CLI wrapper PATH setup requires a POSIX shell",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_probe_path_pulls_value_between_delimiters() {
        let out = format!("some rc banner\n{d}/usr/bin:/bin{d}", d = PROBE_DELIM);
        assert_eq!(extract_probe_path(&out), Some("/usr/bin:/bin"));
    }

    #[test]
    fn extract_probe_path_none_without_delimiters() {
        assert_eq!(extract_probe_path("no delimiters here"), None);
        assert_eq!(
            extract_probe_path(&format!("{}only one", PROBE_DELIM)),
            None
        );
    }

    #[test]
    fn extract_probe_path_handles_empty_path() {
        let out = format!("{d}{d}", d = PROBE_DELIM);
        assert_eq!(extract_probe_path(&out), Some(""));
    }

    #[test]
    fn path_var_contains_matches_exact_entry() {
        let dir = Path::new("/Users/joker/.claudepot/bin");
        assert!(path_var_contains(
            "/usr/bin:/Users/joker/.claudepot/bin:/bin",
            dir
        ));
        assert!(path_var_contains("/Users/joker/.claudepot/bin", dir));
    }

    #[test]
    fn path_var_contains_rejects_absent_or_partial() {
        let dir = Path::new("/Users/joker/.claudepot/bin");
        assert!(!path_var_contains("/usr/bin:/bin", dir));
        // A prefix of the entry must not count as a match.
        assert!(!path_var_contains("/Users/joker/.claudepot", dir));
        assert!(!path_var_contains("", dir));
    }

    #[test]
    fn rc_path_for_shell_maps_known_shells() {
        let home = Path::new("/Users/joker");
        assert_eq!(
            rc_path_for_shell("/bin/zsh", home),
            Some(home.join(".zshrc"))
        );
        assert_eq!(
            rc_path_for_shell("/usr/local/bin/bash", home),
            Some(home.join(".bash_profile"))
        );
    }

    #[test]
    fn rc_path_for_shell_none_for_unsupported() {
        let home = Path::new("/Users/joker");
        assert_eq!(rc_path_for_shell("/opt/homebrew/bin/fish", home), None);
        assert_eq!(rc_path_for_shell("", home), None);
    }

    #[test]
    fn path_entry_forms_covers_home_relative_spellings() {
        let home = Path::new("/Users/joker");
        let dir = Path::new("/Users/joker/.claudepot/bin");
        let forms = path_entry_forms(dir, home);
        // `$HOME/…` is the emitted (first) form.
        assert_eq!(forms[0], "$HOME/.claudepot/bin");
        assert!(forms.contains(&"~/.claudepot/bin".to_string()));
        assert!(forms.contains(&"/Users/joker/.claudepot/bin".to_string()));
    }

    #[test]
    fn path_entry_forms_falls_back_to_absolute_outside_home() {
        // A `CLAUDEPOT_DATA_DIR` override can land outside $HOME.
        let home = Path::new("/Users/joker");
        let dir = Path::new("/tmp/cp-test/bin");
        assert_eq!(path_entry_forms(dir, home), vec!["/tmp/cp-test/bin"]);
    }

    #[test]
    fn path_export_block_includes_marker_and_home_relative_dir() {
        let home = Path::new("/Users/joker");
        let block = path_export_block(Path::new("/Users/joker/.claudepot/bin"), home);
        assert!(block.contains(RC_MARKER));
        assert!(block.contains("export PATH=\"$HOME/.claudepot/bin:$PATH\""));
    }

    #[test]
    fn path_is_shell_safe_accepts_clean_paths_rejects_metachars() {
        assert!(path_is_shell_safe(Path::new("/Users/joker/.claudepot/bin")));
        // Spaces are fine inside the double-quoted export value.
        assert!(path_is_shell_safe(Path::new("/tmp/cp test/bin")));
        for bad in [
            "/tmp/we\"ird/bin",
            "/tmp/we$ird/bin",
            "/tmp/we`ird/bin",
            "/tmp/we\\ird/bin",
            "/tmp/we'ird/bin",
        ] {
            assert!(!path_is_shell_safe(Path::new(bad)), "should reject {bad:?}",);
        }
    }

    #[test]
    fn contains_path_entry_requires_delimited_boundaries() {
        let entry = "/Users/joker/.claudepot/bin";
        // Sibling / child paths must not count as a match.
        assert!(!contains_path_entry(
            "export PATH=\"/Users/joker/.claudepot/bin-old:$PATH\"",
            entry,
        ));
        assert!(!contains_path_entry(
            "export PATH=\"/Users/joker/.claudepot/bin/sub:$PATH\"",
            entry,
        ));
        // Genuine entry, delimited by a quote and a colon.
        assert!(contains_path_entry(
            "export PATH=\"/Users/joker/.claudepot/bin:$PATH\"",
            entry,
        ));
        // Genuine entry at the end of the string.
        assert!(contains_path_entry(
            "PATH=/usr/bin:/Users/joker/.claudepot/bin",
            entry,
        ));
        assert!(!contains_path_entry("", entry));
    }

    #[test]
    fn body_has_wrapper_dir_detects_every_spelling() {
        let home = Path::new("/Users/joker");
        let dir = Path::new("/Users/joker/.claudepot/bin");
        // Our own emitted block — detected via the `$HOME/` form.
        assert!(body_has_wrapper_dir(
            &path_export_block(dir, home),
            dir,
            home
        ));
        // Hand-written references in any spelling.
        assert!(body_has_wrapper_dir(
            "export PATH=\"$HOME/.claudepot/bin:$PATH\"",
            dir,
            home
        ));
        assert!(body_has_wrapper_dir(
            "export PATH=\"~/.claudepot/bin:$PATH\"",
            dir,
            home
        ));
        assert!(body_has_wrapper_dir(
            "export PATH=\"/Users/joker/.claudepot/bin:$PATH\"",
            dir,
            home
        ));
        assert!(!body_has_wrapper_dir("export PATH=\"/usr/bin\"", dir, home));
    }

    #[test]
    fn body_has_wrapper_dir_rejects_marker_without_entry() {
        // A stray marker comment (e.g. a partially-applied hand
        // edit) must NOT suppress a real append — the decision is
        // made on the actual PATH entry, not the comment.
        let home = Path::new("/Users/joker");
        let dir = Path::new("/Users/joker/.claudepot/bin");
        assert!(!body_has_wrapper_dir(
            &format!("{RC_MARKER}\n# (export line was removed by hand)\n"),
            dir,
            home
        ));
    }

    #[test]
    fn is_path_assignment_recognizes_real_assignments_only() {
        for yes in [
            "export PATH=\"$HOME/.claudepot/bin:$PATH\"",
            "PATH=/usr/bin:/bin",
            "  PATH+=:/opt/bin",
            "typeset -x PATH=/usr/bin",
            "path=(/usr/bin /opt/bin)",
        ] {
            assert!(is_path_assignment(yes), "should accept {yes:?}");
        }
        for no in [
            "# export PATH=\"$HOME/.claudepot/bin:$PATH\"",
            "MYPATH=/Users/joker/.claudepot/bin",
            "echo \"my notes about /Users/joker/.claudepot/bin\"",
            // Mentions `PATH=` but inside a command, not as an
            // assignment statement.
            "echo \"PATH=/Users/joker/.claudepot/bin\"",
            "export EDITOR=vim",
        ] {
            assert!(!is_path_assignment(no), "should reject {no:?}");
        }
    }

    #[test]
    fn body_has_wrapper_dir_ignores_comments_and_unrelated_vars() {
        let home = Path::new("/Users/joker");
        let dir = Path::new("/Users/joker/.claudepot/bin");
        // Path mentioned in a comment — not actually on PATH.
        assert!(!body_has_wrapper_dir(
            "# remember to add /Users/joker/.claudepot/bin someday\n",
            dir,
            home
        ));
        // Path in an unrelated variable — not on PATH.
        assert!(!body_has_wrapper_dir(
            "MYDIR=\"/Users/joker/.claudepot/bin\"\n",
            dir,
            home
        ));
        // Path echoed inside a command — not on PATH.
        assert!(!body_has_wrapper_dir(
            "echo \"PATH=/Users/joker/.claudepot/bin\"\n",
            dir,
            home
        ));
        // Path in a trailing inline comment on a real PATH line —
        // the assignment *value* doesn't contain it.
        assert!(!body_has_wrapper_dir(
            "PATH=\"/usr/bin:/bin\" # TODO add /Users/joker/.claudepot/bin\n",
            dir,
            home
        ));
        // Path in a `;`-separated command after a PATH assignment.
        assert!(!body_has_wrapper_dir(
            "PATH=\"/usr/bin:/bin\"; echo /Users/joker/.claudepot/bin\n",
            dir,
            home
        ));
        // A real PATH assignment IS still detected, even with
        // comment lines around it and a trailing inline comment.
        assert!(body_has_wrapper_dir(
            "# claudepot\nexport PATH=\"$HOME/.claudepot/bin:$PATH\" # routes\n",
            dir,
            home
        ));
        // zsh array form is recognized.
        assert!(body_has_wrapper_dir(
            "path=($HOME/.claudepot/bin $path)\n",
            dir,
            home
        ));
    }
}
