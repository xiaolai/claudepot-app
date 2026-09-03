//! Installing a Claudepot verb as a Claude Code hook, and taking it out.
//!
//! Two features hook Claude Code: remote approvals (`remote::approval`,
//! event `PermissionRequest`) and permission grants (`permission::hook`,
//! event `PreToolUse`). Each is one entry in the user's
//! `~/.claude/settings.json` pointing back at this binary. The shape of
//! that entry, and the care around writing it, is the same for both, so
//! it lives here once. A second copy of "find our entry among the
//! user's hooks and replace it" is a second place for the removal to
//! leave litter or eat a stranger's hook.
//!
//! Every write goes through [`crate::settings_mutex`], which is the
//! only sanctioned way to read-modify-write a CC settings file. A
//! second writer doing its own RMW would silently discard whichever
//! mutation landed first.
//!
//! **The exec form (`command` + `args`) is deliberate.** CC resolves
//! `command` as an executable and spawns it directly with these
//! arguments, with no shell anywhere. A binary path containing a space,
//! a quote, a `$` or a backtick therefore never reaches a shell parser
//! — the failure this repo has already paid for once, in a commit
//! message that executed its own backticks.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value as JsonValue};

use crate::paths::claude_config_dir;
use crate::settings_mutex::{self, Change, Mutation, SettingsMutexError};

/// One Claudepot hook: which CC event it answers, the verb that
/// identifies it among the user's own hooks, and how long CC may wait
/// on it before killing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookSpec {
    /// The CC hook event, e.g. `PermissionRequest`.
    pub event: &'static str,
    /// The `claudepot` subcommand, e.g. `["hook", "permission-request"]`.
    /// Matched to recognise our entry regardless of where the binary
    /// lives — see [`is_ours`].
    pub verb: &'static [&'static str],
    /// Written as the entry's `timeout`. CC kills the hook at it, and a
    /// killed `PreToolUse` hook blocks the tool call, so every verb
    /// must finish well inside its own number.
    pub timeout_secs: u64,
}

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

/// Our hook entry for `spec`, pointing at `binary`.
pub fn entry(spec: &HookSpec, binary: &Path) -> JsonValue {
    json!({
        "type": "command",
        "command": binary.to_string_lossy(),
        "args": spec.verb,
        "timeout": spec.timeout_secs,
    })
}

/// Is this one of ours?
///
/// Matched on the verb rather than the binary path, so an entry written
/// by a Claudepot that has since moved or been reinstalled is still
/// recognised as ours to replace or remove — rather than left behind as
/// a stranger's hook we politely decline to touch.
pub fn is_ours(spec: &HookSpec, hook: &JsonValue) -> bool {
    hook.get("args")
        .and_then(|a| a.as_array())
        .is_some_and(|a| {
            a.len() >= spec.verb.len()
                && a.iter()
                    .zip(spec.verb.iter())
                    .all(|(got, want)| got.as_str() == Some(*want))
        })
}

/// Strip our entries for `spec` from a `hooks` object, returning
/// whether anything went. Leaves the user's own entries, and prunes
/// containers we emptied so an uninstall does not leave
/// `"PermissionRequest": []` behind as litter.
pub fn remove_ours(spec: &HookSpec, root: &mut Map<String, JsonValue>) -> bool {
    let Some(hooks) = root.get_mut("hooks").and_then(|h| h.as_object_mut()) else {
        return false;
    };
    let Some(groups) = hooks.get_mut(spec.event).and_then(|g| g.as_array_mut()) else {
        return false;
    };

    let mut changed = false;
    for group in groups.iter_mut() {
        if let Some(list) = group.get_mut("hooks").and_then(|l| l.as_array_mut()) {
            let before = list.len();
            list.retain(|h| !is_ours(spec, h));
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
        hooks.remove(spec.event);
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    changed
}

/// Is an entry of ours for `spec` present, and does it point at
/// `binary`? `Some(true)` = installed and current, `Some(false)` =
/// installed but pointing elsewhere (a moved binary), `None` = absent
/// or unreadable.
pub fn installed_state(spec: &HookSpec, binary: &Path) -> Option<bool> {
    let want = entry(spec, binary);
    let bytes = std::fs::read(user_settings_path()).ok()?;
    let root: JsonValue = serde_json::from_slice(&bytes).ok()?;
    let groups = root.get("hooks")?.get(spec.event)?.as_array()?;
    let ours: Vec<&JsonValue> = groups
        .iter()
        .filter_map(|g| g.get("hooks").and_then(|l| l.as_array()))
        .flatten()
        .filter(|h| is_ours(spec, h))
        .collect();
    if ours.is_empty() {
        return None;
    }
    Some(ours.iter().any(|h| **h == want))
}

/// Add the hook, replacing any earlier copy of ours.
///
/// Idempotent: installing twice leaves exactly one entry, and
/// installing after the binary moved rewrites the path rather than
/// adding a second entry pointing at a file that is gone.
pub fn install(spec: &HookSpec, binary: &Path) -> Result<Mutation<()>, InstallError> {
    let want = entry(spec, binary);
    settings_mutex::mutate_settings_file(&user_settings_path(), |root, _was| {
        // Already exactly right? Then write nothing — an unchanged file
        // keeps its mtime, and a reconcile that runs every tick must
        // not look like someone edited the user's settings.
        let already = root
            .get("hooks")
            .and_then(|h| h.get(spec.event))
            .and_then(|g| g.as_array())
            .is_some_and(|groups| {
                groups.iter().any(|g| {
                    g.get("hooks")
                        .and_then(|l| l.as_array())
                        .is_some_and(|l| l.iter().any(|h| is_ours(spec, h) && *h == want))
                })
            });
        if already {
            return Ok::<_, InstallError>(Change::Skip(()));
        }

        remove_ours(spec, root);
        let hooks = root
            .entry("hooks")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or(InstallError::Malformed { key: "hooks" })?;
        let groups = hooks
            .entry(spec.event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or(InstallError::Malformed { key: spec.event })?;
        // No `matcher`: every call of the event, which is the whole
        // point — a prompt this hook skipped would wait at the keyboard
        // with nothing to say why.
        groups.push(json!({ "hooks": [want.clone()] }));
        Ok(Change::Write(()))
    })
}

/// Take the hook back out. Safe to call when it was never installed.
pub fn uninstall(spec: &HookSpec) -> Result<Mutation<()>, InstallError> {
    settings_mutex::mutate_settings_file(&user_settings_path(), |root, _was| {
        if remove_ours(spec, root) {
            Ok::<_, InstallError>(Change::Write(()))
        } else {
            Ok(Change::Skip(()))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: HookSpec = HookSpec {
        event: "PermissionRequest",
        verb: &["hook", "permission-request"],
        timeout_secs: 120,
    };

    const OTHER: HookSpec = HookSpec {
        event: "PreToolUse",
        verb: &["hook", "pre-tool-use"],
        timeout_secs: 10,
    };

    fn ours() -> JsonValue {
        entry(&SPEC, Path::new("/usr/local/bin/claudepot"))
    }

    fn theirs() -> JsonValue {
        json!({"type": "command", "command": "/bin/echo", "args": ["hi"]})
    }

    #[test]
    fn the_entry_uses_the_exec_form_so_no_shell_ever_sees_the_path() {
        let e = entry(&SPEC, Path::new("/Users/a b/`whoami`/claudepot"));
        assert_eq!(e["command"], json!("/Users/a b/`whoami`/claudepot"));
        assert_eq!(e["args"], json!(["hook", "permission-request"]));
        assert_eq!(e["type"], json!("command"));
        assert_eq!(e["timeout"], json!(120));
    }

    #[test]
    fn ours_is_recognised_by_verb_not_by_path() {
        assert!(is_ours(
            &SPEC,
            &entry(&SPEC, Path::new("/anywhere/else/claudepot"))
        ));
        assert!(!is_ours(&SPEC, &theirs()));
        // A near-miss must not be adopted.
        assert!(!is_ours(
            &SPEC,
            &json!({"args": ["hook", "something-else"]})
        ));
        assert!(!is_ours(&SPEC, &json!({"args": ["hook"]})));
        assert!(!is_ours(
            &SPEC,
            &json!({"command": "claudepot hook permission-request"})
        ));
        // And one feature's entry is not the other's.
        assert!(!is_ours(&SPEC, &entry(&OTHER, Path::new("/x"))));
        assert!(!is_ours(&OTHER, &ours()));
    }

    #[test]
    fn removing_ours_leaves_the_users_own_hooks_alone() {
        let mut root: Map<String, JsonValue> = serde_json::from_value(json!({
            "hooks": {"PermissionRequest": [{"hooks": [theirs(), ours()]}]}
        }))
        .unwrap();

        assert!(remove_ours(&SPEC, &mut root));
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

        assert!(remove_ours(&SPEC, &mut root));
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

        assert!(remove_ours(&SPEC, &mut root));
        assert!(root["hooks"].get("PermissionRequest").is_none());
        assert!(root["hooks"].get("PreToolUse").is_some());
    }

    #[test]
    fn one_features_uninstall_spares_the_others_entry() {
        // Both Claudepot hooks live in the same file. Taking the
        // approval hook out when `remote serve` stops must not take a
        // live permission grant's hook with it.
        let mut root: Map<String, JsonValue> = serde_json::from_value(json!({
            "hooks": {
                "PermissionRequest": [{"hooks": [ours()]}],
                "PreToolUse": [{"hooks": [entry(&OTHER, Path::new("/x"))]}]
            }
        }))
        .unwrap();
        assert!(remove_ours(&SPEC, &mut root));
        assert!(root["hooks"].get("PermissionRequest").is_none());
        assert!(is_ours(&OTHER, &root["hooks"]["PreToolUse"][0]["hooks"][0]));
    }

    #[test]
    fn removing_from_a_file_that_never_had_us_changes_nothing() {
        let mut empty: Map<String, JsonValue> = Map::new();
        assert!(!remove_ours(&SPEC, &mut empty));

        let mut theirs_only: Map<String, JsonValue> = serde_json::from_value(json!({
            "hooks": {"PermissionRequest": [{"hooks": [theirs()]}]}
        }))
        .unwrap();
        assert!(!remove_ours(&SPEC, &mut theirs_only));
        assert_eq!(
            theirs_only["hooks"]["PermissionRequest"][0]["hooks"],
            json!([theirs()])
        );
    }

    /// The file-level behaviour, on a real settings file under an
    /// isolated `CLAUDE_CONFIG_DIR`.
    mod on_disk {
        use super::*;

        fn isolated() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
            let lock = crate::testing::lock_data_dir();
            let tmp = tempfile::tempdir().unwrap();
            std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path());
            (tmp, lock)
        }

        #[test]
        fn install_is_idempotent_and_rewrites_a_moved_binary_path() {
            // A grant can outlive a Homebrew upgrade or an app move. The
            // reconcile that runs every tick must repair the path, not
            // stack a second entry beside one pointing at nothing.
            let (_t, _l) = isolated();
            install(&SPEC, Path::new("/old/claudepot")).unwrap();
            install(&SPEC, Path::new("/old/claudepot")).unwrap();
            assert_eq!(
                installed_state(&SPEC, Path::new("/old/claudepot")),
                Some(true)
            );

            install(&SPEC, Path::new("/new/claudepot")).unwrap();
            assert_eq!(
                installed_state(&SPEC, Path::new("/new/claudepot")),
                Some(true)
            );
            assert_eq!(
                installed_state(&SPEC, Path::new("/old/claudepot")),
                Some(false),
                "the old path is gone, not kept beside the new one"
            );
            let root: JsonValue =
                serde_json::from_slice(&std::fs::read(user_settings_path()).unwrap()).unwrap();
            let entries: Vec<&JsonValue> = root["hooks"]["PermissionRequest"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|g| g["hooks"].as_array().unwrap())
                .collect();
            assert_eq!(entries.len(), 1);
        }

        #[test]
        fn uninstall_removes_only_this_specs_entry() {
            let (_t, _l) = isolated();
            install(&SPEC, Path::new("/x")).unwrap();
            install(&OTHER, Path::new("/x")).unwrap();
            uninstall(&SPEC).unwrap();
            assert_eq!(installed_state(&SPEC, Path::new("/x")), None);
            assert_eq!(installed_state(&OTHER, Path::new("/x")), Some(true));
            uninstall(&OTHER).unwrap();
            assert_eq!(installed_state(&OTHER, Path::new("/x")), None);
            let root: JsonValue =
                serde_json::from_slice(&std::fs::read(user_settings_path()).unwrap()).unwrap();
            assert!(root.get("hooks").is_none(), "{root}");
        }

        #[test]
        fn installed_state_is_none_with_no_settings_file() {
            let (_t, _l) = isolated();
            assert_eq!(installed_state(&SPEC, Path::new("/x")), None);
        }
    }
}
