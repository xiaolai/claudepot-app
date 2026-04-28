//! Pure attribution + parsing helpers for the JSONL extractor.
//!
//! Sharded out of `extract.rs` per the loc-guardian rule "more than 5
//! private fns → extract to `<domain>_helpers.rs`". Lives in the same
//! module so callers don't notice the split (re-exported via
//! `pub(super)` and consumed via `super::`).
//!
//! All functions here are pure — no I/O, no JSON parsing of unknown
//! shapes, no allocations beyond the returned String. They can be
//! tested with literal inputs and benched without setup.

/// Pull every `<command-name>/foo</command-name>` (and the open-only
/// `<command-name>/foo` form CC sometimes emits without the close
/// tag) from a message body. Order-preserving, dedup-free — a user
/// message rarely contains more than one slash command, but the
/// extractor handles N for forward compat.
pub(super) fn extract_slash_commands(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(start) = body[cursor..].find("<command-name>") {
        let abs = cursor + start + "<command-name>".len();
        let rest = &body[abs..];
        // Slice up to the next `<` or end-of-line / end-of-string —
        // whichever closes the command name first.
        let end = rest.find(['<', '\n']).unwrap_or(rest.len());
        let cmd = rest[..end].trim();
        if cmd.starts_with('/') && cmd.len() > 1 {
            out.push(cmd.to_string());
        }
        cursor = abs + end;
    }
    out
}

/// Canonical hook key — `<hookName>|<command>`. Two hooks that share a
/// shell command but fire on different events are distinct artifacts;
/// the previous key (just `command`) merged them.
///
/// Falls back to `hookName` alone when no command is present
/// (hook_cancelled before exec, malformed line). Returns `None` when
/// neither field is usable so the caller can skip the row.
///
/// **UI joins must use this same function** so renderer keys line up
/// with extractor keys exactly.
pub fn hook_artifact_key(hook_name: Option<&str>, command: Option<&str>) -> Option<String> {
    let name = hook_name.filter(|s| !s.is_empty());
    let cmd = command.filter(|s| !s.is_empty());
    match (name, cmd) {
        (Some(n), Some(c)) => Some(format!("{n}|{c}")),
        (Some(n), None) => Some(n.to_string()),
        (None, Some(c)) => Some(c.to_string()),
        (None, None) => None,
    }
}

/// Extract plugin id from a skill `path` value.
/// `plugin:foo:bar` → Some("foo"); `userSettings:bar` → None.
pub(crate) fn parse_skill_plugin_id(path: &str) -> Option<String> {
    let rest = path.strip_prefix("plugin:")?;
    let id = rest.split(':').next()?;
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// Extract plugin id from a colon-prefixed identifier such as a
/// subagent_type (`loc-guardian:counter`) or slash command without
/// the leading slash. The leftmost segment is the plugin id when a
/// colon is present; standalone identifiers (`Explore`) return None.
pub(crate) fn parse_colon_plugin_id(s: &str) -> Option<String> {
    let (head, tail) = s.split_once(':')?;
    if head.is_empty() || tail.is_empty() {
        None
    } else {
        Some(head.to_string())
    }
}

/// Extract plugin id from a slash command name. `/codex-toolkit:audit`
/// → Some("codex-toolkit"); `/clear` → None.
pub(crate) fn parse_command_plugin_id(cmd: &str) -> Option<String> {
    let bare = cmd.strip_prefix('/')?;
    parse_colon_plugin_id(bare)
}

/// Extract plugin id from a hook command line. CC writes hook
/// commands either as `${CLAUDE_PLUGIN_ROOT}/...` (no plugin id
/// visible at write time) or as a fully-resolved
/// `~/.claude/plugins/cache/<owner>/<plugin-id>/...` path. We attempt
/// the latter — when the substring `plugins/cache/<owner>/<id>` is
/// present, return `<id>`. Returns None for env-templated forms.
pub(crate) fn parse_hook_plugin_id(command: &str) -> Option<String> {
    let after = command.split("plugins/cache/").nth(1)?;
    let mut parts = after.splitn(3, '/');
    let _owner = parts.next()?;
    let id = parts.next()?;
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_slash_commands_handles_multiple_in_one_message() {
        let body = "<command-name>/a</command-name>\nthen later <command-name>/b:c</command-name>";
        let cmds = extract_slash_commands(body);
        assert_eq!(cmds, vec!["/a".to_string(), "/b:c".to_string()]);
    }

    #[test]
    fn extract_slash_commands_ignores_non_slash_content() {
        let body = "<command-name>not-a-slash</command-name>";
        assert!(extract_slash_commands(body).is_empty());
    }

    #[test]
    fn extract_slash_commands_ignores_empty_command() {
        let body = "<command-name>/</command-name>";
        assert!(extract_slash_commands(body).is_empty());
    }

    #[test]
    fn parse_skill_plugin_id_variants() {
        assert_eq!(parse_skill_plugin_id("plugin:foo:bar"), Some("foo".into()));
        assert_eq!(parse_skill_plugin_id("plugin:foo"), Some("foo".into()));
        assert_eq!(parse_skill_plugin_id("userSettings:bar"), None);
        assert_eq!(parse_skill_plugin_id("plugin:"), None);
        assert_eq!(parse_skill_plugin_id(""), None);
    }

    #[test]
    fn parse_hook_plugin_id_from_resolved_path() {
        assert_eq!(
            parse_hook_plugin_id(
                "node /Users/me/.claude/plugins/cache/xiaolai/tdd-guardian/0.1.0/scripts/x.js"
            ),
            Some("tdd-guardian".into())
        );
        assert_eq!(
            parse_hook_plugin_id("bash ${CLAUDE_PLUGIN_ROOT}/x.sh"),
            None
        );
        assert_eq!(parse_hook_plugin_id("ls -la"), None);
    }

    #[test]
    fn parse_command_plugin_id_variants() {
        assert_eq!(parse_command_plugin_id("/foo:bar"), Some("foo".into()));
        assert_eq!(parse_command_plugin_id("/clear"), None);
        assert_eq!(parse_command_plugin_id("foo:bar"), None);
    }

    #[test]
    fn hook_artifact_key_helper_handles_all_combinations() {
        assert_eq!(
            hook_artifact_key(Some("PreToolUse:Bash"), Some("node /h.js")),
            Some("PreToolUse:Bash|node /h.js".into())
        );
        assert_eq!(hook_artifact_key(Some("Stop"), None), Some("Stop".into()));
        assert_eq!(hook_artifact_key(None, Some("true")), Some("true".into()));
        assert_eq!(hook_artifact_key(None, None), None);
        assert_eq!(hook_artifact_key(Some(""), Some("")), None);
    }
}
