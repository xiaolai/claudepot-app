//! Help template catalog — see `dev-docs/activity-cards-design.md` §6.
//!
//! Each template is a `(template_id, render_fn)` entry. The render
//! function receives the card's `HelpRef.args` map and returns the
//! final user-facing English. Keeping the text out of SQLite makes
//! catalog edits a code change, not a data migration — and avoids
//! the encoded-backslash pain of JSON template strings.
//!
//! New templates are added here. The classifier (in `classifier.rs`)
//! decides *which* template a given JSONL line maps to; this file
//! decides *what each template renders to*.
//!
//! v1 ships exactly one template (`hook.plugin_missing`). The other
//! eight from the design doc land in Phase 2 — by then we have the
//! full classifier rule set and can add their match conditions and
//! render lines together.

use super::card::HelpRef;

/// Render a help reference to its final English string.
///
/// Returns `None` for unknown template ids — never panics, never
/// fabricates a fallback. A card whose template_id is missing from
/// the catalog renders without a help line, which is the correct
/// behavior on a version mismatch (an older binary reading rows
/// written by a newer one).
pub fn render(help: &HelpRef) -> Option<String> {
    match help.template_id.as_str() {
        "hook.plugin_missing" => Some(render_plugin_missing(help)),
        "hook.json_invalid" => Some(render_json_invalid(help)),
        "tool.read_required" => Some(render_read_required(help)),
        "tool.parallel_cancelled" => Some(render_parallel_cancelled()),
        "tool.ssh_timeout" => Some(render_ssh_timeout(help)),
        "tool.no_such_file" => Some(render_no_such_file(help)),
        "tool.edit_drift" => Some(render_edit_drift(help)),
        "tool.user_rejected" => Some(render_user_rejected()),
        "tool.bash_cmd_not_found" => Some(render_bash_cmd_not_found(help)),
        "agent.no_return" => Some(render_agent_no_return(help)),
        "agent.error_return" => Some(render_agent_error_return(help)),
        _ => None,
    }
}

/// Helper: pull a named arg or fall back to a placeholder. Keeps
/// every render fn one-liner-friendly without `unwrap`.
fn arg<'a>(help: &'a HelpRef, key: &str, fallback: &'a str) -> &'a str {
    help.args.get(key).map(String::as_str).unwrap_or(fallback)
}

/// `hook.plugin_missing` — the single most common hook failure on the
/// reference machine (216 instances out of ~2000 historical failures).
fn render_plugin_missing(help: &HelpRef) -> String {
    format!(
        "Plugin {} is missing. Run /plugin and reinstall.",
        arg(help, "plugin", "the plugin")
    )
}

/// `hook.json_invalid` — CC ignored the hook's `hookSpecificOutput`
/// because the JSON didn't match the expected schema.
fn render_json_invalid(help: &HelpRef) -> String {
    let detail = arg(help, "detail", "");
    if detail.is_empty() {
        "Hook output failed schema validation. CC ignored the directive — check the hook's JSON shape.".into()
    } else {
        format!("Hook output failed schema validation. CC ignored the directive: {detail}")
    }
}

/// `tool.read_required` — model wrote to a file it didn't read first,
/// or read it and it changed in between. CC re-reads and retries
/// automatically; no user action needed.
fn render_read_required(help: &HelpRef) -> String {
    format!(
        "Stale read — model will re-read {} and retry. No action needed.",
        arg(help, "file", "the file")
    )
}

/// `tool.parallel_cancelled` — a sibling tool call in the same
/// parallel batch failed and CC aborted the rest.
fn render_parallel_cancelled() -> String {
    "Parallel tool aborted because a sibling failed. Look at the failing sibling above.".into()
}

/// `tool.ssh_timeout` — SSH connect timed out. Most common cause on
/// this network is a host that's down or behind a NAT.
fn render_ssh_timeout(help: &HelpRef) -> String {
    format!(
        "SSH timeout to {}. Host may be down or unreachable — check tailscale/network status.",
        arg(help, "host", "the host")
    )
}

/// `tool.no_such_file` — referenced path doesn't exist.
fn render_no_such_file(help: &HelpRef) -> String {
    let path = arg(help, "path", "the path");
    let cwd = arg(help, "cwd", "");
    if cwd.is_empty() {
        format!("{path} not found. Check spelling or working directory.")
    } else {
        format!("{path} not found. Check spelling or working directory ({cwd}).")
    }
}

/// `tool.edit_drift` — the target string changed between read and
/// edit. Most often: a hook or another tool modified the file in
/// between.
fn render_edit_drift(help: &HelpRef) -> String {
    format!(
        "Edit failed: target string drifted in {}. Re-read the file and retry.",
        arg(help, "file", "the file")
    )
}

/// `tool.user_rejected` — the user clicked Reject on a tool prompt.
fn render_user_rejected() -> String {
    "You declined this tool call.".into()
}

/// `tool.bash_cmd_not_found` — the bash invocation referenced a
/// binary that isn't on PATH. When the classifier supplied a
/// `brew_install_hint` arg (a known package name), surface it; the
/// design's per-template promise is "one specific next action."
fn render_bash_cmd_not_found(help: &HelpRef) -> String {
    let cmd = arg(help, "command", "command");
    if let Some(pkg) = help.args.get("brew_install_hint") {
        format!("`{cmd}` not installed. Try `brew install {pkg}`.")
    } else {
        format!("`{cmd}` not installed or not on PATH. Install it or use an absolute path.")
    }
}

/// `agent.no_return` — an Agent tool_use was opened in the parent
/// transcript but never produced a matching `tool_result`. Most
/// often: user cancelled mid-flight, or the subagent crashed before
/// returning.
fn render_agent_no_return(_help: &HelpRef) -> String {
    "Agent did not return. Open the subagent transcript for details.".into()
}

/// `agent.error_return` — Agent returned with `is_error: true` set
/// on its tool_result.
fn render_agent_error_return(_help: &HelpRef) -> String {
    "Agent reported an error. Open the return for details.".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn href(id: &str, args: &[(&str, &str)]) -> HelpRef {
        HelpRef {
            template_id: id.to_string(),
            args: args
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn unknown_template_returns_none_not_placeholder() {
        // A future writer may persist a card with a template_id this
        // older binary doesn't know about. The right answer is None
        // (= no help line shown), not a "[unknown template]" string.
        let h = href("hook.future_thing_we_dont_have_yet", &[]);
        assert_eq!(render(&h), None);
    }

    #[test]
    fn plugin_missing_renders_with_plugin_name() {
        let h = href(
            "hook.plugin_missing",
            &[("plugin", "mermaid-preview@xiaolai")],
        );
        assert_eq!(
            render(&h).as_deref(),
            Some("Plugin mermaid-preview@xiaolai is missing. Run /plugin and reinstall.")
        );
    }

    #[test]
    fn plugin_missing_renders_without_plugin_name() {
        // Falls back to a neutral phrasing — never panics, never
        // shows raw "{plugin}" placeholder text to the user.
        let h = href("hook.plugin_missing", &[]);
        assert_eq!(
            render(&h).as_deref(),
            Some("Plugin the plugin is missing. Run /plugin and reinstall.")
        );
    }

    /// Pin every Phase 2 template's exact rendered text. Asserting
    /// verbatim catches accidental rewording — these strings ship
    /// to users.
    #[test]
    fn phase_2_templates_render_verbatim() {
        let cases = [
            (
                href("hook.json_invalid", &[("detail", "missing field 'continue'")]),
                "Hook output failed schema validation. CC ignored the directive: missing field 'continue'",
            ),
            (
                href("hook.json_invalid", &[]),
                "Hook output failed schema validation. CC ignored the directive — check the hook's JSON shape.",
            ),
            (
                href("tool.read_required", &[("file", "/x/y.rs")]),
                "Stale read — model will re-read /x/y.rs and retry. No action needed.",
            ),
            (
                href("tool.parallel_cancelled", &[]),
                "Parallel tool aborted because a sibling failed. Look at the failing sibling above.",
            ),
            (
                href("tool.ssh_timeout", &[("host", "192.0.2.7")]),
                "SSH timeout to 192.0.2.7. Host may be down or unreachable — check tailscale/network status.",
            ),
            (
                href("tool.no_such_file", &[("path", "/tmp/missing"), ("cwd", "/x")]),
                "/tmp/missing not found. Check spelling or working directory (/x).",
            ),
            (
                href("tool.no_such_file", &[("path", "x.txt")]),
                "x.txt not found. Check spelling or working directory.",
            ),
            (
                href("tool.edit_drift", &[("file", "src/x.rs")]),
                "Edit failed: target string drifted in src/x.rs. Re-read the file and retry.",
            ),
            (
                href("tool.user_rejected", &[]),
                "You declined this tool call.",
            ),
            (
                href("tool.bash_cmd_not_found", &[("command", "fzf")]),
                "`fzf` not installed or not on PATH. Install it or use an absolute path.",
            ),
            (
                href(
                    "tool.bash_cmd_not_found",
                    &[("command", "rg"), ("brew_install_hint", "ripgrep")],
                ),
                "`rg` not installed. Try `brew install ripgrep`.",
            ),
            (
                href("agent.no_return", &[]),
                "Agent did not return. Open the subagent transcript for details.",
            ),
            (
                href("agent.error_return", &[]),
                "Agent reported an error. Open the return for details.",
            ),
        ];
        for (h, expected) in cases {
            assert_eq!(
                render(&h).as_deref(),
                Some(expected),
                "template {} render mismatch",
                h.template_id
            );
        }
    }
}
