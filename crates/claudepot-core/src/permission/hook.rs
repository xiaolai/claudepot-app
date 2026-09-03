//! What the `PreToolUse` hook decides from a grant.
//!
//! Claude Code runs `claudepot hook pre-tool-use` before every tool
//! call while a grant is live, and hands it the session's working
//! directory. If a live grant covers that directory the hook answers
//! `allow` and CC skips the permission prompt; otherwise it prints
//! nothing, which CC reads as "decide as usual".
//!
//! **Why `PreToolUse` and not the `PermissionRequest` hook remote
//! approvals use.** Measured on the 2.1.259 binary (2026-09-03), in
//! real interactive sessions:
//!
//! | mode   | `PermissionRequest` allow            | `PreToolUse` allow                |
//! |--------|--------------------------------------|-----------------------------------|
//! | Manual | fires once, prompt skipped           | fires once, prompt skipped        |
//! | auto   | never fires; the classifier decided  | fires once, **classifier skipped**|
//!
//! Auto mode is the built-in starting mode on Pro, Max and Team, and
//! there `PermissionRequest` only fires for the few prompts the
//! classifier still draws — so a grant built on it would do nothing
//! for most users. `PreToolUse` runs before the permission system
//! decides anything, so an `allow` from it is what `bypassPermissions`
//! used to be: no prompt, and no 2–3 s classifier round trip per call.
//! Deny and ask rules still apply — CC evaluates them regardless of a
//! hook's answer — and so does everything no mode auto-approves.
//!
//! The cost is one process spawn per tool call while a grant is live,
//! reads included: ~13 ms for this binary, measured. There is
//! deliberately no `matcher`: a fixed list of "tools that can prompt"
//! would drift with CC, and a prompt this hook skipped is a prompt at
//! the keyboard with nothing on screen saying why.
//!
//! Two further properties are load-bearing:
//!
//! - **The hook never touches the file.** It runs inside every tool
//!   call on the machine, as the user, from a process CC started.
//!   [`load_readonly`] therefore parses and nothing else: no
//!   corruption recovery, no rename-aside, no log line. A corrupt file
//!   reads as "no grant", the call goes through CC's normal permission
//!   flow, and the GUI's next tick is what moves the file aside and
//!   tells the user. Recovering from inside the hook would race the
//!   GUI for the same rename.
//!
//! - **Scope is the session's working directory, by path components.**
//!   A session counts as inside a project when its `cwd` is the
//!   project root or a descendant. That is the same scope
//!   `bypassPermissions` had — per session, not per file the session
//!   touches — so a granted session that runs `cd ../elsewhere && …`
//!   is auto-approved exactly as it would have been under bypass mode.
//!   A subagent reports its parent session's `cwd` and is covered too
//!   (measured).
//!
//! The entry lives in `~/.claude/settings.json` beside the remote
//! approval hook's, is installed while any grant is live and removed
//! when the last one lapses ([`reconcile`]), and is re-pointed at the
//! current binary on every orchestrator tick so a grant outliving an
//! app upgrade keeps working.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::cc_hook_entry::{self, HookSpec, InstallError};
use crate::path_utils::{canonicalize_simplified, is_absolute_path_str, simplify_windows_path};
use crate::permission::grants::{Grant, GrantsFile};

/// What we write into CC's settings as the entry's `timeout`. The
/// grant path does no waiting — one file read — so ten seconds is a
/// ceiling on a wedged disk, not a budget. CC kills a hook at its
/// timeout and a killed `PreToolUse` hook **blocks the tool call**, so
/// this must stay comfortably above the verb's real cost (~13 ms).
pub const HOOK_TIMEOUT_SECS: u64 = 10;

/// The entry: `PreToolUse`, every tool, our hidden verb.
pub const SPEC: HookSpec = HookSpec {
    event: "PreToolUse",
    verb: &["hook", "pre-tool-use"],
    timeout_secs: HOOK_TIMEOUT_SECS,
};

/// The one field of CC's `PreToolUse` payload this hook reads.
/// `serde` ignores the rest, so a new field upstream is not a parse
/// failure — CC ships ~27 releases a month.
#[derive(Debug, Clone, Deserialize)]
pub struct PreToolUseInput {
    #[serde(default)]
    pub cwd: String,
}

/// What the hook prints on stdout for CC to skip the prompt.
///
/// Shape verified live against 2.1.259: CC's debug log reads it back as
/// `returned permissionDecision: allow (reason: …)` and the call runs
/// with no dialog and no classifier request.
pub fn decision_output() -> serde_json::Value {
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "permissionDecisionReason": "Claudepot permission grant for this project",
        }
    })
}

/// The live grant covering `cwd` at `now`, if any.
///
/// Pure: the caller reads the file. An empty or relative `cwd` never
/// matches — CC always sends an absolute one, and a payload without it
/// is a payload this hook does not understand well enough to answer.
pub fn covering_grant<'a>(
    file: &'a GrantsFile,
    cwd: &str,
    now: DateTime<Utc>,
) -> Option<&'a Grant> {
    file.grants
        .iter()
        .filter(|g| !g.is_expired(now))
        .find(|g| path_is_within(cwd, &g.project_path))
}

/// Does the hook entry need to exist at all? True while any grant is
/// live. `remote::service::reconcile_permission_hook` ORs this with
/// the remote-approval gate.
pub fn any_live_grant(file: &GrantsFile, now: DateTime<Utc>) -> bool {
    file.grants.iter().any(|g| !g.is_expired(now))
}

/// [`covering_grant`], then once more with both sides canonicalized
/// when the literal comparison found nothing.
///
/// CC's `cwd` is `process.cwd()`, already resolved (`/tmp` arrives as
/// `/private/tmp` on macOS), and Claudepot's project paths come from
/// CC's own transcripts, so the literal comparison is the normal hit.
/// The resolved pass covers a project registered through a symlink;
/// it costs a few `stat` calls and only runs on a miss.
pub fn covering_grant_resolved<'a>(
    file: &'a GrantsFile,
    cwd: &str,
    now: DateTime<Utc>,
) -> Option<&'a Grant> {
    if let Some(g) = covering_grant(file, cwd, now) {
        return Some(g);
    }
    let cwd = canonicalize_simplified(Path::new(cwd)).ok()?;
    let cwd = cwd.to_string_lossy();
    file.grants.iter().filter(|g| !g.is_expired(now)).find(|g| {
        canonicalize_simplified(Path::new(&g.project_path))
            .ok()
            .is_some_and(|root| path_is_within(&cwd, &root.to_string_lossy()))
    })
}

/// Make the user's `settings.json` agree with the grants file: the
/// entry is present, pointing at `binary`, iff a grant is live.
/// Returns whether it is now installed.
///
/// Idempotent and cheap when nothing changed (an unchanged file is
/// not rewritten), so the orchestrator calls it every tick — which is
/// also what repairs the path after the binary moves, since
/// `cc_hook_entry::install` rewrites a stale `command`.
pub fn reconcile(
    file: &GrantsFile,
    now: DateTime<Utc>,
    binary: &Path,
) -> Result<bool, InstallError> {
    if any_live_grant(file, now) {
        cc_hook_entry::install(&SPEC, binary)?;
        Ok(true)
    } else {
        cc_hook_entry::uninstall(&SPEC)?;
        Ok(false)
    }
}

/// Is our entry present and pointing at `binary`? See
/// `cc_hook_entry::installed_state` for the three answers.
pub fn installed_state(binary: &Path) -> Option<bool> {
    cc_hook_entry::installed_state(&SPEC, binary)
}

/// Is `child` equal to `parent` or a descendant of it, by path
/// components?
///
/// Both must be absolute. Separators are compared per platform: a
/// backslash is a separator on Windows and an ordinary filename byte on
/// Unix. Windows comparisons are case-insensitive, matching the
/// filesystem; a `\\?\` verbatim prefix is stripped first because CC
/// never writes one and `canonicalize` on Windows always does. A
/// trailing separator is not a component, so `/p/a/` and `/p/a` are
/// the same root — and `/p/a-evil` is not under `/p/a`, which a string
/// prefix test would have said it was.
pub fn path_is_within(child: &str, parent: &str) -> bool {
    if child.is_empty() || parent.is_empty() {
        return false;
    }
    if !is_absolute_path_str(child) || !is_absolute_path_str(parent) {
        return false;
    }
    let (child, parent) = (simplify_windows_path(child), simplify_windows_path(parent));
    let (child, parent) = (components(&child), components(&parent));
    if parent.is_empty() || child.len() < parent.len() {
        return false;
    }
    child
        .iter()
        .zip(parent.iter())
        .all(|(c, p)| same_component(c, p))
}

fn components(path: &str) -> Vec<&str> {
    let is_sep = |ch: char| ch == '/' || (cfg!(windows) && ch == '\\');
    path.split(is_sep).filter(|c| !c.is_empty()).collect()
}

fn same_component(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// Parse the grants file without recovery, without logging, and
/// without treating a missing file as anything but empty.
///
/// `None` for every failure — absent, unreadable, unparseable, or a
/// schema newer than this binary — which the hook reads as "no grant".
/// See the module docs for why this must not share the store's
/// recovery path.
pub fn load_readonly(path: &Path) -> Option<GrantsFile> {
    let bytes = std::fs::read(path).ok()?;
    let file: GrantsFile = serde_json::from_slice(&bytes).ok()?;
    file.validate().ok()?;
    Some(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn grant(path: &str, expires: Option<i64>) -> Grant {
        Grant {
            project_path: path.into(),
            granted_at: ts(0),
            expires_at: expires.map(ts),
        }
    }

    fn file(grants: Vec<Grant>) -> GrantsFile {
        GrantsFile {
            grants,
            ..GrantsFile::default()
        }
    }

    // ── path_is_within — the four shapes rules/paths.md requires ──

    #[test]
    fn unix_root_and_descendants_are_within() {
        assert!(path_is_within("/Users/a/proj", "/Users/a/proj"));
        assert!(path_is_within("/Users/a/proj/", "/Users/a/proj"));
        assert!(path_is_within("/Users/a/proj/src/deep", "/Users/a/proj/"));
    }

    #[test]
    fn a_sibling_sharing_a_string_prefix_is_not_within() {
        // `/p/a-evil` starts with `/p/a`; a byte-prefix test would
        // have granted it.
        assert!(!path_is_within("/Users/a/proj-evil", "/Users/a/proj"));
        assert!(!path_is_within("/Users/a/projects", "/Users/a/proj"));
    }

    #[test]
    fn a_parent_is_not_within_its_child() {
        assert!(!path_is_within("/Users/a", "/Users/a/proj"));
        assert!(!path_is_within("/", "/Users/a/proj"));
    }

    #[test]
    fn relative_or_empty_paths_never_match() {
        assert!(!path_is_within("", "/Users/a/proj"));
        assert!(!path_is_within("/Users/a/proj", ""));
        assert!(!path_is_within("proj/src", "/Users/a/proj"));
        assert!(!path_is_within("/Users/a/proj/src", "proj"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_drive_paths_compare_by_component_and_case_insensitively() {
        assert!(path_is_within(r"C:\Users\a\proj\src", r"C:\Users\a\proj"));
        assert!(path_is_within(r"c:\users\A\PROJ", r"C:\Users\a\proj"));
        assert!(!path_is_within(r"C:\Users\a\proj-evil", r"C:\Users\a\proj"));
        assert!(!path_is_within(r"D:\Users\a\proj", r"C:\Users\a\proj"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_unc_and_verbatim_paths_are_within_their_plain_form() {
        assert!(path_is_within(
            r"\\server\share\proj\src",
            r"\\server\share\proj"
        ));
        // `canonicalize` yields the verbatim form; CC never writes it.
        assert!(path_is_within(
            r"\\?\C:\Users\a\proj\src",
            r"C:\Users\a\proj"
        ));
        assert!(path_is_within(
            r"\\?\UNC\server\share\proj",
            r"\\server\share\proj"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_backslash_is_a_filename_byte_on_unix() {
        // Splitting on `\` here would make `/p/a\b` look like `/p/a`.
        assert!(!path_is_within(r"/p/a\b", "/p/a"));
        assert!(path_is_within(r"/p/a\b/c", r"/p/a\b"));
    }

    // ── covering_grant / any_live_grant ────────────────────────────

    #[test]
    fn a_live_grant_covers_its_root_and_subdirectories() {
        let f = file(vec![grant("/p/a", Some(100))]);
        assert!(covering_grant(&f, "/p/a", ts(50)).is_some());
        assert!(covering_grant(&f, "/p/a/src", ts(50)).is_some());
        assert!(covering_grant(&f, "/p/b", ts(50)).is_none());
    }

    #[test]
    fn an_expired_grant_covers_nothing() {
        let f = file(vec![grant("/p/a", Some(100))]);
        assert!(covering_grant(&f, "/p/a", ts(100)).is_none());
        assert!(!any_live_grant(&f, ts(100)));
        assert!(any_live_grant(&f, ts(99)));
    }

    #[test]
    fn a_sticky_grant_covers_forever() {
        let f = file(vec![grant("/p/a", None)]);
        assert!(covering_grant(&f, "/p/a/x", ts(i32::MAX as i64)).is_some());
        assert!(any_live_grant(&f, ts(i32::MAX as i64)));
    }

    #[test]
    fn an_empty_file_grants_nothing() {
        let f = file(vec![]);
        assert!(covering_grant(&f, "/p/a", ts(0)).is_none());
        assert!(!any_live_grant(&f, ts(0)));
    }

    // ── load_readonly — silence on every failure ───────────────────

    #[test]
    fn load_readonly_reads_a_good_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("g.json");
        let f = file(vec![grant("/p/a", None)]);
        std::fs::write(&p, serde_json::to_vec(&f).unwrap()).unwrap();
        assert_eq!(load_readonly(&p), Some(f));
    }

    #[test]
    fn load_readonly_never_moves_a_corrupt_file_aside() {
        // The store's recovery renames the file; the hook must not,
        // because it runs inside every prompt as the user and would
        // race the GUI for the rename.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("g.json");
        std::fs::write(&p, b"{not json").unwrap();
        assert!(load_readonly(&p).is_none());
        assert_eq!(std::fs::read(&p).unwrap(), b"{not json");
        assert!(crate::json_store::corrupt_siblings(&p).is_empty());
    }

    #[test]
    fn load_readonly_refuses_a_schema_it_does_not_know() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("g.json");
        std::fs::write(&p, br#"{"schema_version":99,"grants":[]}"#).unwrap();
        assert!(load_readonly(&p).is_none());
    }

    #[test]
    fn load_readonly_of_a_missing_file_is_none() {
        assert!(load_readonly(Path::new("/nonexistent/claudepot/grants.json")).is_none());
    }

    // ── covering_grant_resolved — symlinked roots ──────────────────

    #[cfg(unix)]
    #[test]
    fn a_grant_registered_through_a_symlink_still_covers_the_real_path() {
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("real");
        std::fs::create_dir_all(real.join("src")).unwrap();
        let link = d.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let real_canon = canonicalize_simplified(&real).unwrap();

        let f = file(vec![grant(link.to_str().unwrap(), None)]);
        // Literal comparison misses; the resolved pass hits.
        let cwd = real_canon.join("src");
        assert!(covering_grant(&f, cwd.to_str().unwrap(), ts(0)).is_none());
        assert!(covering_grant_resolved(&f, cwd.to_str().unwrap(), ts(0)).is_some());
        // And an unrelated real directory is still not covered.
        let other = d.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        let other = canonicalize_simplified(&other).unwrap();
        assert!(covering_grant_resolved(&f, other.to_str().unwrap(), ts(0)).is_none());
    }

    // ── reconcile — the entry follows the grants ───────────────────

    mod on_disk {
        use super::*;

        fn isolated() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
            let lock = crate::testing::lock_data_dir();
            let tmp = tempfile::tempdir().unwrap();
            std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path());
            (tmp, lock)
        }

        #[test]
        fn the_entry_exists_exactly_while_a_grant_is_live() {
            let (_t, _l) = isolated();
            let bin = Path::new("/opt/claudepot");
            let f = file(vec![grant("/p/a", Some(100))]);

            assert!(reconcile(&f, ts(50), bin).unwrap());
            assert_eq!(installed_state(bin), Some(true));

            // Same grant, past its deadline: the entry goes.
            assert!(!reconcile(&f, ts(100), bin).unwrap());
            assert_eq!(installed_state(bin), None);

            // No grants at all, never installed: still nothing, no error.
            assert!(!reconcile(&file(vec![]), ts(0), bin).unwrap());
            assert_eq!(installed_state(bin), None);
        }

        #[test]
        fn reconcile_repairs_a_stale_binary_path() {
            // The sticky-grant-outlives-an-upgrade case: the tick's
            // reconcile must re-point the entry, not leave one aimed at
            // a binary that is gone.
            let (_t, _l) = isolated();
            let f = file(vec![grant("/p/a", None)]);
            reconcile(&f, ts(0), Path::new("/old/claudepot")).unwrap();
            reconcile(&f, ts(0), Path::new("/new/claudepot")).unwrap();
            assert_eq!(installed_state(Path::new("/new/claudepot")), Some(true));
            assert_eq!(installed_state(Path::new("/old/claudepot")), Some(false));
        }
    }

    #[test]
    fn the_decision_is_the_pre_tool_use_shape_cc_honours() {
        let v = decision_output();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
    }

    #[test]
    fn the_timeout_is_a_ceiling_far_above_the_verbs_cost_and_under_ccs_clamp() {
        assert!(SPEC.timeout_secs >= 5);
        assert!(SPEC.timeout_secs < 300);
        assert_eq!(SPEC.event, "PreToolUse");
    }
}
