// Settings → MCP pane (WI-009).
//
// Two halves:
//
// 1. Server health card — calls shared_memory_mcp_health, shows
//    "tool_visible" status. The current binary acts as both server
//    and probe; spawn-and-list verifies the stdio handshake works.
//
// 2. Snippet installer — write
//    ~/.claude/claudepot-mcp-instructions.md (or chosen path) and
//    print the @include line for the user to paste into CLAUDE.md
//    / AGENTS.md. Includes a preview pane.
//
// The plan's "dual-signal health" framing (tool_visible AND
// workflow_installed) is partially honored: tool_visible is
// queryable; workflow_installed is necessarily a user-attestation
// (we can't read CLAUDE.md / AGENTS.md across projects without
// guessing). UI shows the snippet path as a confirmation.

import { useCallback, useEffect, useMemo, useState } from "react";
import { projectApi } from "../../api/project";
import { sharedMemoryApi } from "../../api/sharedMemory";
import type {
  McpHealth,
  SnippetInstallResult,
  SnippetScope,
} from "../../api/sharedMemory";
import type { ProjectInfo } from "../../types/project";
import { Button } from "../../components/primitives/Button";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";

// Narrow toast signature to the only two kinds we use, keeping the
// pane decoupled from the broader Toast type. The PushToast prop
// is a function reference, so the caller's wider signature is
// structurally assignable.
type PushToast = (kind: "info" | "error", text: string) => void;

export function McpInstallerPane({
  pushToast,
}: {
  pushToast: PushToast;
}) {
  const [health, setHealth] = useState<McpHealth | null>(null);
  const [checking, setChecking] = useState(false);
  const [snippet, setSnippet] = useState<string>("");
  const [install, setInstall] = useState<SnippetInstallResult | null>(null);
  const [installing, setInstalling] = useState(false);
  const [scope, setScope] = useState<SnippetScope>("user");
  const [projectPath, setProjectPath] = useState<string>("");
  const [projects, setProjects] = useState<ProjectInfo[]>([]);

  const checkHealth = useCallback(async () => {
    setChecking(true);
    try {
      const h = await sharedMemoryApi.mcpHealth();
      setHealth(h);
    } catch (e) {
      setHealth({ tool_visible: false, tool_count: 0, error: String(e) });
    } finally {
      setChecking(false);
    }
  }, []);

  const loadSnippet = useCallback(async () => {
    try {
      const body = await sharedMemoryApi.snippetBody();
      setSnippet(body);
    } catch (e) {
      pushToast("error", `Failed to load snippet preview: ${e}`);
    }
  }, [pushToast]);

  const doInstall = useCallback(async () => {
    setInstalling(true);
    try {
      const r = await sharedMemoryApi.installSnippet(
        scope === "project"
          ? { scope: "project", project_path: projectPath }
          : { scope: "user" },
      );
      setInstall(r);
      pushToast("info", `Wrote ${r.path} (${r.bytes_written} bytes)`);
    } catch (e) {
      pushToast("error", `Install failed: ${e}`);
    } finally {
      setInstalling(false);
    }
  }, [pushToast, scope, projectPath]);

  const loadProjects = useCallback(async () => {
    try {
      const list = await projectApi.projectList();
      setProjects(list.filter((p: ProjectInfo) => p.is_reachable));
    } catch (e) {
      pushToast("error", `Failed to load projects: ${e}`);
    }
  }, [pushToast]);

  const copyIncludeLine = useCallback(async () => {
    if (!install) return;
    try {
      await navigator.clipboard.writeText(install.include_line);
      pushToast("info", "@include line copied to clipboard");
    } catch (e) {
      pushToast("error", `Copy failed: ${e}`);
    }
  }, [install, pushToast]);

  useEffect(() => {
    void loadSnippet();
  }, [loadSnippet]);

  useEffect(() => {
    void loadProjects();
  }, [loadProjects]);

  const installDisabled = useMemo(() => {
    if (installing) return true;
    if (scope === "project" && !projectPath) return true;
    return false;
  }, [installing, scope, projectPath]);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 24, maxWidth: 760 }}>
      {/* ─── Server health ─────────────────────────────────── */}
      <section>
        <SectionLabel>Server health</SectionLabel>
        <div
          style={{
            marginTop: 8,
            padding: 16,
            border: "tokens.sp.px solid var(--line)",
            borderRadius: 8,
            background: "var(--bg-raised)",
            display: "flex",
            flexDirection: "column",
            gap: 12,
          }}
        >
          <div style={{ display: "flex", gap: 12, alignItems: "center" }}>
            <Tag>
              {health
                ? health.tool_visible
                  ? `tool_visible · ${health.tool_count} tools`
                  : "tool_visible · failed"
                : "unknown"}
            </Tag>
            <div style={{ flex: 1, color: "var(--fg-muted)", fontSize: "var(--fs-sm)" }}>
              Spawns <code>claudepot mcp memory-server</code>, runs
              <code> initialize</code> + <code>tools/list</code>, counts the tools.
            </div>
            <Button glyph={NF.refresh} onClick={() => void checkHealth()} disabled={checking}>
              {checking ? "Checking…" : "Check"}
            </Button>
          </div>
          {health?.error && (
            <pre
              style={{
                margin: 0,
                padding: 10,
                background: "var(--bg-sunken)",
                borderRadius: 6,
                fontSize: "var(--fs-2xs, tokens.sp[12])",
                color: "var(--danger)",
                whiteSpace: "pre-wrap",
              }}
            >
              {health.error}
            </pre>
          )}
        </div>
      </section>

      {/* ─── Agent-instruction snippet installer ──────────── */}
      <section>
        <SectionLabel>Agent instructions</SectionLabel>
        <p style={{ marginTop: 6, fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}>
          Configuring an MCP entry alone isn't enough — agents have to
          be told <em>when</em> to call the tools. This snippet does
          that. Pick a scope, install it once, then paste the{" "}
          <code>@include</code> line into the target file Claudepot
          names. Future regenerations refresh the file in place; the{" "}
          <code>@include</code> line never changes.
        </p>
        <div
          style={{
            marginTop: 8,
            padding: 16,
            border: "tokens.sp.px solid var(--line)",
            borderRadius: 8,
            background: "var(--bg-raised)",
            display: "flex",
            flexDirection: "column",
            gap: 12,
          }}
        >
          {/* Scope picker */}
          <fieldset
            style={{
              border: "none",
              padding: 0,
              margin: 0,
              display: "flex",
              flexDirection: "column",
              gap: 6,
            }}
          >
            <legend style={{ fontSize: "var(--fs-sm)", color: "var(--fg-muted)", padding: 0 }}>
              Scope
            </legend>
            <label style={{ display: "flex", gap: 8, alignItems: "flex-start", cursor: "pointer" }}>
              <input
                type="radio"
                name="snippet-scope"
                value="user"
                checked={scope === "user"}
                onChange={() => setScope("user")}
              />
              <span>
                <strong>User</strong>{" "}
                <span style={{ color: "var(--fg-muted)" }}>
                  — writes <code>~/.claude/claudepot-mcp-instructions.md</code>; paste{" "}
                  <code>@include</code> into <code>~/.claude/CLAUDE.md</code>,{" "}
                  <code>~/.codex/AGENTS.md</code>, <code>~/.gemini/GEMINI.md</code>. Loads
                  in every project.
                </span>
              </span>
            </label>
            <label style={{ display: "flex", gap: 8, alignItems: "flex-start", cursor: "pointer" }}>
              <input
                type="radio"
                name="snippet-scope"
                value="project"
                checked={scope === "project"}
                onChange={() => setScope("project")}
              />
              <span>
                <strong>Project</strong>{" "}
                <span style={{ color: "var(--fg-muted)" }}>
                  — writes <code>&lt;project&gt;/.claude/claudepot-mcp-instructions.md</code>;
                  paste <code>@include</code> into the project's <code>AGENTS.md</code> (the
                  canonical source per <code>/init-workspace</code>). Loads only in that
                  project.
                </span>
              </span>
            </label>
          </fieldset>
          {scope === "project" && (
            <select
              value={projectPath}
              onChange={(e) => setProjectPath(e.currentTarget.value)}
              aria-label="Project"
              style={{
                padding: 8,
                border: "tokens.sp.px solid var(--line)",
                borderRadius: 6,
                background: "var(--bg-base)",
                fontFamily: "monospace",
                fontSize: "var(--fs-sm)",
              }}
            >
              <option value="">Select project…</option>
              {projects.map((p) => (
                <option key={p.original_path} value={p.original_path}>
                  {p.original_path}
                </option>
              ))}
            </select>
          )}
          <div style={{ display: "flex", gap: 12 }}>
            <Button
              variant="solid"
              glyph={NF.download}
              onClick={() => void doInstall()}
              disabled={installDisabled}
            >
              {installing ? "Installing…" : "Install snippet"}
            </Button>
            {install && (
              <Button glyph={NF.copy} onClick={() => void copyIncludeLine()}>
                Copy @include line
              </Button>
            )}
          </div>
          {install && (
            <div
              style={{
                padding: 10,
                background: "var(--bg-sunken)",
                borderRadius: 6,
                fontSize: "var(--fs-2xs, tokens.sp[12])",
                fontFamily: "monospace",
                userSelect: "text",
              }}
            >
              <div>
                Wrote ({install.scope}): {install.path}
              </div>
              <div style={{ marginTop: 4, fontWeight: 600 }}>{install.include_line}</div>
              {install.target_files.length > 0 && (
                <div style={{ marginTop: 6, color: "var(--fg-muted)" }}>
                  Paste the line above into:
                  <ul style={{ margin: "tokens.sp[4] 0 0 tokens.sp[16]", padding: 0 }}>
                    {install.target_files.map((f) => (
                      <li key={f}>{f}</li>
                    ))}
                  </ul>
                </div>
              )}
            </div>
          )}
          <details style={{ marginTop: 4 }}>
            <summary style={{ cursor: "pointer", fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}>
              Preview snippet content
            </summary>
            <pre
              style={{
                marginTop: 8,
                padding: 12,
                background: "var(--bg-sunken)",
                borderRadius: 6,
                maxHeight: 360,
                overflow: "auto",
                fontSize: "var(--fs-2xs, tokens.sp[12])",
                whiteSpace: "pre-wrap",
                userSelect: "text",
              }}
            >
              {snippet}
            </pre>
          </details>
        </div>
      </section>
    </div>
  );
}
