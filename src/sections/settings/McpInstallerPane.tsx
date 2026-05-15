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

import { useCallback, useEffect, useState } from "react";
import { sharedMemoryApi } from "../../api/sharedMemory";
import type { McpHealth, SnippetInstallResult } from "../../api/sharedMemory";
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
      const r = await sharedMemoryApi.installSnippet();
      setInstall(r);
      pushToast("info", `Wrote ${r.path} (${r.bytes_written} bytes)`);
    } catch (e) {
      pushToast("error", `Install failed: ${e}`);
    } finally {
      setInstalling(false);
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
          that. Install it once, then paste the <code>@include</code>{" "}
          line into your <code>CLAUDE.md</code> (and/or{" "}
          <code>AGENTS.md</code>). Future regenerations refresh the
          file in place; the <code>@include</code> line never changes.
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
          <div style={{ display: "flex", gap: 12 }}>
            <Button variant="solid" glyph={NF.download} onClick={() => void doInstall()} disabled={installing}>
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
              <div>Wrote: {install.path}</div>
              <div style={{ marginTop: 4, fontWeight: 600 }}>{install.include_line}</div>
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
