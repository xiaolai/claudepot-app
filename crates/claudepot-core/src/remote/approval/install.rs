//! Wiring the hook into Claude Code's settings, and taking it back out.
//!
//! **Installation is coupled to the remote surface's own switch.** The
//! ability to approve a tool call from a phone exists only while the
//! thing that reaches the phone is switched on, so `remote serve`
//! installs on start and removes on stop. That coupling is the reason
//! this feature is acceptable at all: it does not quietly widen what
//! the machine will do on an install that never turns remote on.
//!
//! It is the *outer* half of the gate, and the weaker one — settings
//! files are hand-edited, apps are force-quit, and an entry can outlive
//! the process that wrote it. [`super::store::gate`] is the half that
//! holds when this one has failed.
//!
//! Every write goes through [`crate::settings_mutex`], which is the
//! only sanctioned way to read-modify-write a CC settings file. A
//! second writer doing its own RMW would silently discard whichever
//! mutation landed first.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value as JsonValue};

use crate::paths::claude_config_dir;
use crate::settings_mutex::{self, Change, Mutation, SettingsMutexError};

/// The event we hook. CC fires it exactly when it is about to draw a
/// permission prompt — unlike `PreToolUse`, which fires on every call
/// including ones already allowed, and would pause work nobody was
/// being asked about.
const EVENT: &str = "PermissionRequest";

/// The verb that identifies our entry among the user's own hooks.
const VERB: [&str; 2] = ["hook", "permission-request"];

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("settings: {0}")]
    Settings(#[from] SettingsMutexError),
    #[error("serialize hook entry: {0}")]
    Json(#[from] serde_json::Error),
    /// `hooks` (or the event under it) exists and is not the shape CC
    /// defines. Refused rather than overwritten: it is the user's file,
    /// and silently replacing a key we cannot read is how an editor
    /// eats a config.
    #[error("{}: `{key}` is not the expected shape — fix it by hand, or remove it", crate::paths::claude_config_dir().join("settings.json").display())]
    Malformed { key: &'static str },
}

fn user_settings_path() -> PathBuf {
    claude_config_dir().join("settings.json")
}

/// Our hook entry.
///
/// The **exec form** (`command` + `args`) is deliberate: CC resolves
/// `command` as an executable and spawns it directly with these
/// arguments, with no shell anywhere. A binary path containing a space,
/// a quote, a `$` or a backtick therefore never reaches a shell parser
/// — the failure this repo has already paid for once, in a commit
/// message that executed its own backticks.
fn entry(binary: &Path) -> JsonValue {
    json!({
        "type": "command",
        "command": binary.to_string_lossy(),
        "args": VERB,
        "timeout": super::HOOK_TIMEOUT_SECS,
    })
}

/// Is this one of ours?
///
/// Matched on the verb rather than the binary path, so an entry written
/// by a Claudepot that has since moved or been reinstalled is still
/// recognised as ours to replace or remove — rather than left behind as
/// a stranger's hook we politely decline to touch.
fn is_ours(hook: &JsonValue) -> bool {
    hook.get("args")
        .and_then(|a| a.as_array())
        .is_some_and(|a| {
            a.len() >= VERB.len()
                && a.iter()
                    .zip(VERB.iter())
                    .all(|(got, want)| got.as_str() == Some(*want))
        })
}

/// Strip our entries from a `hooks` object, returning whether anything
/// went. Leaves the user's own entries, and prunes containers we
/// emptied so an uninstall does not leave `"PermissionRequest": []`
/// behind as litter.
fn remove_ours(root: &mut Map<String, JsonValue>) -> bool {
    let Some(hooks) = root.get_mut("hooks").and_then(|h| h.as_object_mut()) else {
        return false;
    };
    let Some(groups) = hooks.get_mut(EVENT).and_then(|g| g.as_array_mut()) else {
        return false;
    };

    let mut changed = false;
    for group in groups.iter_mut() {
        if let Some(list) = group.get_mut("hooks").and_then(|l| l.as_array_mut()) {
            let before = list.len();
            list.retain(|h| !is_ours(h));
            changed |= list.len() != before;
        }
    }
    // A group whose only hook was ours is now an empty group.
    let before = groups.len();
    groups.retain(|g| {
        g.get("hooks")
            .and_then(|l| l.as_array())
            .is_none_or(|l| !l.is_empty())
    });
    changed |= groups.len() != before;

    if groups.is_empty() {
        hooks.remove(EVENT);
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    changed
}

/// Add the hook, replacing any earlier copy of ours.
///
/// Idempotent: installing twice leaves exactly one entry, and
/// installing after the binary moved rewrites the path rather than
/// adding a second entry pointing at a file that is gone.
pub fn install(binary: &Path) -> Result<Mutation<()>, InstallError> {
    let want = entry(binary);
    settings_mutex::mutate_settings_file(&user_settings_path(), |root, _was| {
        // Already exactly right? Then write nothing — an unchanged file
        // keeps its mtime, and `remote serve` restarting must not look
        // like someone edited the user's settings.
        let already = root
            .get("hooks")
            .and_then(|h| h.get(EVENT))
            .and_then(|g| g.as_array())
            .is_some_and(|groups| {
                groups.iter().any(|g| {
                    g.get("hooks")
                        .and_then(|l| l.as_array())
                        .is_some_and(|l| l.iter().any(|h| is_ours(h) && *h == want))
                })
            });
        if already {
            return Ok::<_, InstallError>(Change::Skip(()));
        }

        remove_ours(root);
        let hooks = root
            .entry("hooks")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or(InstallError::Malformed { key: "hooks" })?;
        let groups = hooks
            .entry(EVENT)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or(InstallError::Malformed { key: EVENT })?;
        // No `matcher`: every permission prompt, which is the whole
        // point — a prompt this hook skipped would wait at the keyboard
        // with nothing on the phone to say so.
        groups.push(json!({ "hooks": [want.clone()] }));
        Ok(Change::Write(()))
    })
}

/// Take the hook back out. Safe to call when it was never installed.
pub fn uninstall() -> Result<Mutation<()>, InstallError> {
    settings_mutex::mutate_settings_file(&user_settings_path(), |root, _was| {
        if remove_ours(root) {
            Ok::<_, InstallError>(Change::Write(()))
        } else {
            Ok(Change::Skip(()))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ours() -> JsonValue {
        entry(Path::new("/usr/local/bin/claudepot"))
    }

    fn theirs() -> JsonValue {
        json!({"type": "command", "command": "/bin/echo", "args": ["hi"]})
    }

    #[test]
    fn the_entry_uses_the_exec_form_so_no_shell_ever_sees_the_path() {
        let e = entry(Path::new("/Users/a b/`whoami`/claudepot"));
        assert_eq!(e["command"], json!("/Users/a b/`whoami`/claudepot"));
        assert_eq!(e["args"], json!(["hook", "permission-request"]));
        assert_eq!(e["type"], json!("command"));
    }

    #[test]
    fn the_installed_timeout_outlasts_the_wait() {
        assert_eq!(
            entry(Path::new("/x"))["timeout"],
            json!(super::super::HOOK_TIMEOUT_SECS)
        );
        assert!(super::super::WAIT.as_secs() < super::super::HOOK_TIMEOUT_SECS);
    }

    #[test]
    fn ours_is_recognised_by_verb_not_by_path() {
        assert!(is_ours(&entry(Path::new("/anywhere/else/claudepot"))));
        assert!(!is_ours(&theirs()));
        // A near-miss must not be adopted.
        assert!(!is_ours(&json!({"args": ["hook", "something-else"]})));
        assert!(!is_ours(&json!({"args": ["hook"]})));
        assert!(!is_ours(
            &json!({"command": "claudepot hook permission-request"})
        ));
    }

    #[test]
    fn removing_ours_leaves_the_users_own_hooks_alone() {
        let mut root: Map<String, JsonValue> = serde_json::from_value(json!({
            "hooks": {"PermissionRequest": [{"hooks": [theirs(), ours()]}]}
        }))
        .unwrap();

        assert!(remove_ours(&mut root));
        assert_eq!(
            root["hooks"]["PermissionRequest"][0]["hooks"],
            json!([theirs()])
        );
    }

    #[test]
    fn an_uninstall_leaves_no_litter_behind() {
        // The whole `hooks` key was ours; it should be gone, not an
        // empty husk that reads as configuration.
        let mut root: Map<String, JsonValue> = serde_json::from_value(json!({
            "model": "opus",
            "hooks": {"PermissionRequest": [{"hooks": [ours()]}]}
        }))
        .unwrap();

        assert!(remove_ours(&mut root));
        assert!(!root.contains_key("hooks"), "{root:?}");
        assert_eq!(root["model"], json!("opus"), "and nothing else was touched");
    }

    #[test]
    fn other_events_survive_our_uninstall() {
        let mut root: Map<String, JsonValue> = serde_json::from_value(json!({
            "hooks": {
                "PermissionRequest": [{"hooks": [ours()]}],
                "PreToolUse": [{"hooks": [theirs()]}]
            }
        }))
        .unwrap();

        assert!(remove_ours(&mut root));
        assert!(root["hooks"].get("PermissionRequest").is_none());
        assert!(root["hooks"].get("PreToolUse").is_some());
    }

    #[test]
    fn removing_from_a_file_that_never_had_us_changes_nothing() {
        let mut empty: Map<String, JsonValue> = Map::new();
        assert!(!remove_ours(&mut empty));

        let mut theirs_only: Map<String, JsonValue> = serde_json::from_value(json!({
            "hooks": {"PermissionRequest": [{"hooks": [theirs()]}]}
        }))
        .unwrap();
        assert!(!remove_ours(&mut theirs_only));
        assert_eq!(
            theirs_only["hooks"]["PermissionRequest"][0]["hooks"],
            json!([theirs()])
        );
    }
}
