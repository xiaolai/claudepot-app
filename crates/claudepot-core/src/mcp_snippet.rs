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
//!
//! The *install* operation (path policy, validation, write) also
//! lives here — see [`install`]. It was previously implemented
//! twice (CLI verb + Tauri command) and had already drifted the
//! same way the body once did.

use std::path::{Path, PathBuf};

/// Bump this when the snippet body changes meaningfully. The
/// version stamp is the first comment line of the body.
///
/// v2 (Phase 2): added the "Drafting a Claudepot agent" section
/// teaching the `claudepot agent draft` CLI verb and the
/// human-only install gate.
///
/// v3: project confinement. v2 told the agent to call
/// `claudepot_search_memory(query)` with no scope, which searched
/// *every* indexed project — on a real machine that means unrelated
/// client work and personal transcripts. The server now confines
/// itself (see `shared_memory::scope`) and the text no longer
/// encourages a global reach. v3 also drops v2's "don't ask for
/// permission" line on `claudepot_remember`: memories are proposals
/// for the user to review, not facts an agent asserts.
pub const SNIPPET_VERSION: &str = "3";

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

  Searches are **confined to the current project**. The index also
  holds this user's *other* projects; you cannot reach them, and you
  should not try. If you get a `scope_denied` error, that is the
  boundary working as intended — do not attempt to route around it,
  and do not ask the user to disable it.

- **When the user states a durable fact, preference, or pattern**
  ("I always run tests with X", "this project uses Y over Z"),
  call `claudepot_remember(scope="project",
  project_path=cwd(), kind="preference"|"fact"|"pattern"|...,
  content="...", created_by="<your-agent-id>")`.

  What you write is a **proposal**, not a fact. The user reviews it
  before it becomes binding. So record what the evidence supports and
  nothing more: a wrong memory that survives review is worse than no
  memory at all, because it will be trusted.

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
"#,
        version = SNIPPET_VERSION,
    )
}

// ─── install ──────────────────────────────────────────────────

/// File name of the installed snippet — identical in user and
/// project scope so the `@`-import line is predictable.
pub const SNIPPET_FILE_NAME: &str = "claudepot-mcp-instructions.md";

/// Where [`install`] writes the snippet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallScope {
    /// `<claude_config_dir>/claudepot-mcp-instructions.md`.
    /// Honors `$CLAUDE_CONFIG_DIR`; defaults to `~/.claude/`, next
    /// to the user's CLAUDE.md so the `@`-import resolves where CC
    /// actually loads memory from.
    User,
    /// `<project_root>/.claude/claudepot-mcp-instructions.md`.
    Project,
}

impl InstallScope {
    /// Wire string used by the CLI output and the Tauri DTO.
    pub fn as_str(self) -> &'static str {
        match self {
            InstallScope::User => "user",
            InstallScope::Project => "project",
        }
    }
}

/// Result of a successful [`install`].
#[derive(Debug, Clone)]
pub struct InstallReport {
    pub scope: InstallScope,
    /// Where the snippet was written.
    pub path: PathBuf,
    pub bytes_written: usize,
    /// The CC `@`-import line to paste into the target files.
    /// Absolute for user scope and `out`; project-relative
    /// (`@.claude/…`) for project scope.
    pub include_line: String,
    /// Files the user is expected to paste `include_line` into.
    /// User scope: the three agent home configs that auto-load
    /// every session. Project scope: only `AGENTS.md` (CLAUDE.md /
    /// GEMINI.md are `@AGENTS.md` re-exports and shouldn't be
    /// hand-edited). Empty for `out` — we can't know the layout
    /// around an arbitrary path.
    pub target_files: Vec<PathBuf>,
}

/// Failures from [`install`].
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("could not resolve home directory")]
    NoHomeDir,
    #[error("project_path required for scope = \"project\"")]
    MissingProjectPath,
    #[error("project path must be absolute: {}", .0.display())]
    ProjectPathNotAbsolute(PathBuf),
    #[error("project path is not an existing directory: {}", .0.display())]
    ProjectPathNotDir(PathBuf),
    #[error("create parent of {}: {source}", .path.display())]
    CreateParent {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("write {}: {source}", .path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Write the canonical snippet and report where it landed plus the
/// `@`-import line and paste targets. The single install policy for
/// both the CLI's `claudepot mcp install-snippet` verb and the
/// Tauri Settings → MCP installer — the two previously implemented
/// this independently and drifted (project scope, target hints,
/// include-line format, `CLAUDE_CONFIG_DIR` handling).
///
/// * `out` is the power-user escape hatch: write to exactly this
///   path, bypassing `scope` / `project_root`. No validation —
///   trusted callers (a typed CLI flag) only. The Tauri command
///   layer additionally requires renderer-supplied paths to be
///   absolute before delegating here.
/// * Project scope validates `project_root` at the codebase's
///   defense-in-depth level for write paths: absolute AND an
///   existing directory (`<root>/.claude/` is created if missing,
///   the root itself never is).
///
/// Idempotent — re-running overwrites with the current canonical
/// content.
pub fn install(
    scope: InstallScope,
    project_root: Option<&Path>,
    out: Option<&Path>,
) -> Result<InstallReport, InstallError> {
    let (path, target_files, include_line) = if let Some(out) = out {
        (out.to_path_buf(), Vec::new(), format!("@{}", out.display()))
    } else {
        match scope {
            InstallScope::User => {
                // The codex/gemini paste targets live under the
                // home dir; without a resolvable home there is
                // nowhere sane to install. Fail loud.
                let home = dirs::home_dir().ok_or(InstallError::NoHomeDir)?;
                let config_dir = crate::paths::claude_config_dir();
                let path = config_dir.join(SNIPPET_FILE_NAME);
                let include_line = format!("@{}", path.display());
                let target_files = vec![
                    config_dir.join("CLAUDE.md"),
                    home.join(".codex").join("AGENTS.md"),
                    home.join(".gemini").join("GEMINI.md"),
                ];
                (path, target_files, include_line)
            }
            InstallScope::Project => {
                let root = project_root.ok_or(InstallError::MissingProjectPath)?;
                if !root.is_absolute() {
                    return Err(InstallError::ProjectPathNotAbsolute(root.to_path_buf()));
                }
                if !root.is_dir() {
                    return Err(InstallError::ProjectPathNotDir(root.to_path_buf()));
                }
                let path = root.join(".claude").join(SNIPPET_FILE_NAME);
                // CC's @-import syntax inside a markdown file, not
                // a filesystem operation — the forward slash is
                // correct on every host OS. AGENTS.md sits at the
                // project root, so the relative form resolves.
                let include_line = format!("@.claude/{SNIPPET_FILE_NAME}");
                let target_files = vec![root.join("AGENTS.md")];
                (path, target_files, include_line)
            }
        }
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| InstallError::CreateParent {
            path: path.clone(),
            source,
        })?;
    }
    let body = snippet_body();
    std::fs::write(&path, &body).map_err(|source| InstallError::Write {
        path: path.clone(),
        source,
    })?;

    Ok(InstallReport {
        scope,
        path,
        bytes_written: body.len(),
        include_line,
        target_files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_install_out_writes_snippet_and_reports_import_line() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("custom.md");
        let report = install(InstallScope::User, None, Some(&out)).unwrap();
        assert_eq!(report.path, out);
        assert_eq!(report.bytes_written, snippet_body().len());
        assert_eq!(report.include_line, format!("@{}", out.display()));
        assert!(report.target_files.is_empty(), "out scope has no targets");
        assert_eq!(std::fs::read_to_string(&out).unwrap(), snippet_body());
        // Idempotent — second run overwrites cleanly.
        install(InstallScope::User, None, Some(&out)).unwrap();
    }

    #[test]
    fn test_install_project_scope_writes_under_dot_claude() {
        let tmp = tempfile::tempdir().unwrap();
        let report = install(InstallScope::Project, Some(tmp.path()), None).unwrap();
        assert_eq!(
            report.path,
            tmp.path().join(".claude").join(SNIPPET_FILE_NAME)
        );
        assert!(report.path.is_file());
        assert_eq!(
            report.include_line,
            format!("@.claude/{SNIPPET_FILE_NAME}"),
            "project import line is project-relative"
        );
        assert_eq!(report.target_files, vec![tmp.path().join("AGENTS.md")]);
    }

    #[test]
    fn test_install_project_scope_rejects_relative_root() {
        let err = install(
            InstallScope::Project,
            Some(std::path::Path::new("relative/dir")),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, InstallError::ProjectPathNotAbsolute(_)));
    }

    #[test]
    fn test_install_project_scope_rejects_missing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let gone = tmp.path().join("nope");
        let err = install(InstallScope::Project, Some(&gone), None).unwrap_err();
        assert!(matches!(err, InstallError::ProjectPathNotDir(_)));
    }

    #[test]
    fn test_install_project_scope_requires_root() {
        let err = install(InstallScope::Project, None, None).unwrap_err();
        assert!(matches!(err, InstallError::MissingProjectPath));
    }

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
    fn snippet_version_is_three() {
        // Pinned so a future edit to the body without a version bump
        // is caught — users `@include` a file that regenerates in
        // place, and the stamp is how they tell which text they have.
        assert_eq!(SNIPPET_VERSION, "3");
    }

    #[test]
    fn the_snippet_never_tells_an_agent_to_search_across_projects() {
        // v2 said: call `claudepot_search_memory(query)` — with no
        // scope, that searched every project the user had ever opened.
        // The server now confines itself, but the *text* must not
        // encourage reaching past the boundary either, and it must
        // tell the agent what a scope_denied means so it doesn't try
        // to route around it.
        let body = snippet_body();
        assert!(
            body.contains("confined to the current project"),
            "the snippet must state that searches are project-confined"
        );
        assert!(
            body.contains("scope_denied"),
            "the snippet must name the error so an agent doesn't treat it as a bug to work around"
        );
        assert!(
            !body.contains("Don't ask for\n  permission"),
            "v2's 'don't ask for permission' line contradicts the review gate: \
             a memory is a proposal, not a fact"
        );
    }
}
