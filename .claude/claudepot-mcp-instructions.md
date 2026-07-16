<!-- claudepot-mcp-instructions v5 — managed by `claudepot mcp install-snippet` or Claudepot GUI -->

# Claudepot shared memory

You have access to the Claudepot MCP memory server via tools
prefixed `claudepot_*`. Use them. Specifically:

- **At the start of a session in a project**, call
  `claudepot_list_decisions(status="active")` and
  `claudepot_list_memories(scope="project")`. The server is confined
  to this project, so you do NOT need to pass `project_path` — it is
  scoped for you, and passing a path that doesn't exactly match the
  server's project would be refused. These return durable facts and
  design decisions that earlier sessions (yours or another agent's)
  recorded for this project. Treat them as load-bearing context.

  Memories arrive **accepted**: unreviewed distiller proposals and
  rejected claims never appear — only human-reviewed lessons and facts
  recorded directly (via `claudepot_remember`) from a user statement.
  Pass `include_suspect=true` to also see lessons whose anchored code
  has since changed — those carry `review_state="suspect"`; treat them
  as possibly stale, not as truth.

- **Before asking the user a question that history might
  answer**, call `claudepot_search_memory(query)` to look for
  prior conversations on the topic. If a hit looks relevant,
  follow up with `claudepot_read_conversation` on its file_path
  to read the excerpt. Cross-harness — Codex transcripts surface
  alongside Claude transcripts.

  Searches are **confined to the current project**. The index also
  holds this user's *other* projects; you cannot reach them, and you
  should not try. If you get a `scope_denied` error, that is the
  boundary working as intended — do not attempt to route around it,
  and do not ask the user to disable it.

- **When the user states a durable fact, preference, or pattern**
  ("I always run tests with X", "this project uses Y over Z"),
  call `claudepot_remember(scope="project",
  kind="preference"|"fact"|"pattern"|..., content="...",
  created_by="<your-agent-id>")`. You do NOT need `project_path` —
  a confined server fills its own project in. Record only what the
  user actually stated — this becomes active project memory that future
  sessions treat as true, so a wrong entry is worse than none because it
  will be trusted. If a memory turns out wrong or obsolete, retract it
  with `claudepot_archive_memory(id)` and record the corrected fact.

- **When you commit to a non-trivial design decision** with the
  user (data model choice, library pick, architectural cut),
  call `claudepot_log_decision(topic="...", decision="...",
  rationale="...", created_by="...")` — `project_path` is filled in
  for you. If the new decision replaces an older one, pass
  `supersedes_id`.

- **At the START of an audit / fix loop**, call
  `claudepot_list_evidence()` — it returns what prior runs already
  found, fixed, and how it was verified, so you don't re-litigate
  resolved findings. **At the END of the loop**, record your own run:
  `claudepot_submit_evidence(summary="...", verification="...",
  files_changed='["src/a.rs","src/b.rs"]', confidence=N,
  created_by="...")` (`project_path` is filled in for you).

- **To read the provenance links recorded on a durable row**, call
  `claudepot_memory_links(memory_id=...)` (or `decision_id` /
  `evidence_id`). It returns explicitly-recorded links — e.g. the
  evidence supporting a decision — and an empty list when none were
  recorded. A file/exchange link is readable via
  `claudepot_read_conversation`.

- **For discovery**, `claudepot_list_sessions()` and
  `claudepot_list_projects()` enumerate the cache without
  needing a search query. Both are confined to the current project.

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
