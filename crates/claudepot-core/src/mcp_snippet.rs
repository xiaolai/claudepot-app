//! Canonical agent-instruction snippet that tells Claude Code,
//! Codex, and other tool-using agents how to use the Claudepot MCP
//! memory tools.
//!
//! Owned here in `claudepot-core` so the CLI's
//! `claudepot mcp install-snippet` verb and the Tauri Settings →
//! MCP installer pane both emit the same bytes. Previously each
//! had its own copy and they had drifted within a single release.
//!
//! The version stamp at the top of the body lets us update the
//! snippet over time without leaving the user puzzled about why
//! their `@include`'d copy disagrees with a future installer's
//! output — a regen overwrites in place, and they `@include` from
//! a fixed path.

/// Bump this when the snippet body changes meaningfully. The
/// version stamp is the first comment line of the body.
///
/// v2 (Phase 2): added the "Drafting a Claudepot agent" section
/// teaching the `claudepot agent draft` CLI verb and the
/// human-only install gate.
pub const SNIPPET_VERSION: &str = "2";

/// Returns the canonical instruction snippet body. Pure: no I/O,
/// no env reads.
pub fn snippet_body() -> String {
    format!(
        r#"<!-- claudepot-mcp-instructions v{version} — managed by `claudepot mcp install-snippet` or Claudepot GUI -->

# Claudepot shared memory

You have access to the Claudepot MCP memory server via tools
prefixed `claudepot_*`. Use them. Specifically:

- **At the start of a session in a project**, call
  `claudepot_list_decisions(project_path=cwd(), status="active")`
  and `claudepot_list_memories(scope="project", project_path=cwd())`.
  These return durable facts and design decisions that earlier
  sessions (either yours or another agent's) recorded for this
  project. Treat them as load-bearing context.

- **Before asking the user a question that history might
  answer**, call `claudepot_search_memory(query)` to look for
  prior conversations on the topic. If a hit looks relevant,
  follow up with `claudepot_read_conversation` on its file_path
  to read the excerpt. Cross-harness — Codex transcripts surface
  alongside Claude transcripts.

- **When the user states a durable fact, preference, or pattern**
  ("I always run tests with X", "this project uses Y over Z"),
  call `claudepot_remember(scope="project",
  project_path=cwd(), kind="preference"|"fact"|"pattern"|...,
  content="...", created_by="<your-agent-id>")`. Don't ask for
  permission; you can always archive later if it turns out to be
  wrong.

- **When you commit to a non-trivial design decision** with the
  user (data model choice, library pick, architectural cut),
  call `claudepot_log_decision(project_path=cwd(),
  topic="...", decision="...", rationale="...", created_by="...")`.
  If the new decision replaces an older one, pass `supersedes_id`.

- **At the end of an audit / fix loop** where you found and
  resolved problems, call `claudepot_submit_evidence(
  project_path=cwd(), summary="...", verification="...",
  files_changed='["src/a.rs","src/b.rs"]', confidence=N,
  created_by="...")`. Future audit-fix runs in this project will
  see what was already fixed and won't re-litigate.

- **For discovery**, `claudepot_list_sessions(project_path=cwd())`
  and `claudepot_list_projects()` enumerate the cache without
  needing a search query.

All `created_by` ids should identify YOU (e.g.
`codex-cli@2026-05-16`, `claude-code@2026-05-16`,
`<agent-name>@<date>`) so the user can trust the audit trail.

# Drafting a Claudepot agent

A Claudepot **agent** is a scheduled or on-demand `claude -p` run
with a fixed spec (model, tools, prompt, permission mode,
trigger). When you notice a recurring task the user would benefit
from automating, you can **draft** an agent for it from the
command line — via the Bash tool — with `claudepot agent draft`.

- **A draft is inert.** `claudepot agent draft` writes a record
  with `lifecycle = draft`. It does NOT schedule anything, does
  NOT create any OS scheduler artifact, and will NOT run. A draft
  sits in Claudepot waiting for a human.

- **Only a human can install (arm) a draft.** You cannot. There
  is deliberately no `claudepot agent install` verb and no
  `claudepot agent edit` verb. After you draft an agent, **tell
  the user to open Claudepot → Agents and choose "Review &
  install"** to review the spec and arm it. That review click is
  the security gate — never imply the agent is already running.

- **To draft**, run one of:

  ```
  claudepot agent draft --name <slug> --cwd <dir> --prompt "<task>" \
    --drafted-by "<your-agent-id>"
  ```

  or pass a JSON spec on stdin / from a file:

  ```
  claudepot agent draft --from-json spec.json --name <slug> \
    --cwd <dir> --drafted-by "<your-agent-id>"
  ```

  The JSON may be Claudepot-native (`name`, `cwd`, `prompt`,
  `permission_mode`, `allowed_tools`, `trigger`, …) **or** the
  SDK `AgentDefinition` shape (`description`, `prompt`, `tools`,
  `model`, `mcpServers`) — Claudepot normalizes either. Always
  pass `--drafted-by` with an id that identifies YOU so the audit
  trail is honest.

- **Attach Claudepot's own memory server to the draft** with the
  `--attach-memory` flag. This wires the drafted agent to the
  same `claudepot mcp memory-server` you are using now, so the
  agent it produces can read the project's durable decisions and
  memories when it runs. Use it whenever the drafted agent would
  benefit from project context.

- **Inspect existing agents** with `claudepot agent list` and
  `claudepot agent show <id-or-name>` (both read-only).

The snippet you're reading is generated by `claudepot mcp
install-snippet` (or by Claudepot's Settings → MCP installer);
running either again refreshes the content. Your CLAUDE.md /
AGENTS.md should `@include` it once and never duplicate it
inline.
"#,
        version = SNIPPET_VERSION,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_body_contains_version_stamp() {
        let body = snippet_body();
        assert!(
            body.starts_with(&format!(
                "<!-- claudepot-mcp-instructions v{SNIPPET_VERSION}"
            )),
            "snippet must start with the version-stamped comment line"
        );
    }

    #[test]
    fn snippet_body_names_canonical_tools() {
        let body = snippet_body();
        for tool in [
            "claudepot_list_decisions",
            "claudepot_list_memories",
            "claudepot_search_memory",
            "claudepot_read_conversation",
            "claudepot_remember",
            "claudepot_log_decision",
            "claudepot_submit_evidence",
            "claudepot_list_sessions",
            "claudepot_list_projects",
        ] {
            assert!(
                body.contains(tool),
                "snippet body must name `{tool}` so agents know it exists"
            );
        }
    }

    #[test]
    fn snippet_body_teaches_the_agent_draft_verb() {
        // Phase 2: the snippet must name the `agent draft` verb so
        // an AI client knows it exists, and must state that a draft
        // is inert until a human installs it (the security gate).
        let body = snippet_body();
        assert!(
            body.contains("claudepot agent draft"),
            "snippet must name the `claudepot agent draft` verb"
        );
        assert!(
            body.contains("lifecycle = draft") || body.contains("inert"),
            "snippet must teach that a draft is inert / not yet armed"
        );
        assert!(
            body.contains("Review & install") || body.contains("install"),
            "snippet must tell the AI to ask the user to review/install"
        );
        assert!(
            body.contains("--attach-memory"),
            "snippet must teach attaching Claudepot's memory server to a draft"
        );
    }

    #[test]
    fn snippet_version_is_two() {
        // Phase 2 bumped the snippet to v2. Pin it so a future edit
        // to the body without a version bump is caught.
        assert_eq!(SNIPPET_VERSION, "2");
    }
}
