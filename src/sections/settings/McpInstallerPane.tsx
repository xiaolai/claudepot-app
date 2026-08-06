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
import { Trans, useTranslation } from "react-i18next";
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
import { renderError } from "../../lib/i18n-error";

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
  const { t } = useTranslation("settings");
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
      setHealth({ tool_visible: false, tool_count: 0, error: renderError(e) });
    } finally {
      setChecking(false);
    }
  }, []);

  const loadSnippet = useCallback(async () => {
    try {
      const body = await sharedMemoryApi.snippetBody();
      setSnippet(body);
    } catch (e) {
      pushToast("error", renderError(e, t("mcp.snippetLoadFailed")));
    }
  }, [pushToast, t]);

  const doInstall = useCallback(async () => {
    setInstalling(true);
    try {
      const r = await sharedMemoryApi.installSnippet(
        scope === "project"
          ? { scope: "project", project_path: projectPath }
          : { scope: "user" },
      );
      setInstall(r);
      pushToast(
        "info",
        t("mcp.wroteToast", { path: r.path, bytes: r.bytes_written }),
      );
    } catch (e) {
      pushToast("error", renderError(e, t("mcp.installFailed")));
    } finally {
      setInstalling(false);
    }
  }, [pushToast, scope, projectPath, t]);

  const loadProjects = useCallback(async () => {
    try {
      const list = await projectApi.projectList();
      setProjects(list.filter((p: ProjectInfo) => p.is_reachable));
    } catch (e) {
      pushToast("error", renderError(e, t("mcp.projectsLoadFailed")));
    }
  }, [pushToast, t]);

  const copyIncludeLine = useCallback(async () => {
    if (!install) return;
    try {
      await navigator.clipboard.writeText(install.include_line);
      pushToast("info", t("mcp.includeCopied"));
    } catch (e) {
      pushToast("error", renderError(e, t("shared.copyFailed")));
    }
  }, [install, pushToast, t]);

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
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-24)", maxWidth: "var(--content-cap-md)" }}>
      {/* ─── Server health ─────────────────────────────────── */}
      <section>
        <SectionLabel>{t("mcp.serverHealth")}</SectionLabel>
        <div
          style={{
            marginTop: "var(--sp-8)",
            padding: "var(--sp-16)",
            border: "var(--sp-px) solid var(--line)",
            borderRadius: "var(--r-3)",
            background: "var(--bg-raised)",
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-12)",
          }}
        >
          <div style={{ display: "flex", gap: "var(--sp-12)", alignItems: "center" }}>
            <Tag>
              {health
                ? health.tool_visible
                  ? t("mcp.toolVisibleCount", { count: health.tool_count })
                  : t("mcp.toolVisibleFailed")
                : t("mcp.unknown")}
            </Tag>
            <div style={{ flex: 1, color: "var(--fg-muted)", fontSize: "var(--fs-sm)" }}>
              <Trans
                ns="settings"
                i18nKey="mcp.healthDesc"
                components={{ code: <code /> }}
              />
            </div>
            <Button glyph={NF.refresh} onClick={() => void checkHealth()} disabled={checking}>
              {checking ? t("mcp.checking") : t("mcp.check")}
            </Button>
          </div>
          {health?.error && (
            <pre
              style={{
                margin: 0,
                padding: "var(--sp-10)",
                background: "var(--bg-sunken)",
                borderRadius: "var(--r-2)",
                fontSize: "var(--fs-2xs)",
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
        <SectionLabel>{t("mcp.agentInstructions")}</SectionLabel>
        <p style={{ marginTop: "var(--sp-6)", fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}>
          <Trans
            ns="settings"
            i18nKey="mcp.introDesc"
            components={{ em: <em />, code: <code /> }}
          />
        </p>
        <div
          style={{
            marginTop: "var(--sp-8)",
            padding: "var(--sp-16)",
            border: "var(--sp-px) solid var(--line)",
            borderRadius: "var(--r-3)",
            background: "var(--bg-raised)",
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-12)",
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
              gap: "var(--sp-6)",
            }}
          >
            <legend style={{ fontSize: "var(--fs-sm)", color: "var(--fg-muted)", padding: 0 }}>
              {t("mcp.scope")}
            </legend>
            <label style={{ display: "flex", gap: "var(--sp-8)", alignItems: "flex-start", cursor: "pointer" }}>
              <input
                type="radio"
                name="snippet-scope"
                value="user"
                checked={scope === "user"}
                onChange={() => setScope("user")}
              />
              <span>
                <strong>{t("mcp.scopeUser")}</strong>{" "}
                <span style={{ color: "var(--fg-muted)" }}>
                  <Trans
                    ns="settings"
                    i18nKey="mcp.scopeUserDesc"
                    components={{ code: <code /> }}
                  />
                </span>
              </span>
            </label>
            <label style={{ display: "flex", gap: "var(--sp-8)", alignItems: "flex-start", cursor: "pointer" }}>
              <input
                type="radio"
                name="snippet-scope"
                value="project"
                checked={scope === "project"}
                onChange={() => setScope("project")}
              />
              <span>
                <strong>{t("mcp.scopeProject")}</strong>{" "}
                <span style={{ color: "var(--fg-muted)" }}>
                  <Trans
                    ns="settings"
                    i18nKey="mcp.scopeProjectDesc"
                    components={{ code: <code /> }}
                    values={{
                      path: "<project>/.claude/claudepot-mcp-instructions.md",
                    }}
                  />
                </span>
              </span>
            </label>
          </fieldset>
          {scope === "project" && (
            <select
              value={projectPath}
              onChange={(e) => setProjectPath(e.currentTarget.value)}
              aria-label={t("mcp.projectAria")}
              style={{
                padding: "var(--sp-8)",
                border: "var(--sp-px) solid var(--line)",
                borderRadius: "var(--r-2)",
                background: "var(--bg-base)",
                fontFamily: "var(--font-mono)",
                fontSize: "var(--fs-sm)",
              }}
            >
              <option value="">{t("mcp.selectProject")}</option>
              {projects.map((p) => (
                <option key={p.original_path} value={p.original_path}>
                  {p.original_path}
                </option>
              ))}
            </select>
          )}
          <div style={{ display: "flex", gap: "var(--sp-12)" }}>
            <Button
              variant="solid"
              glyph={NF.download}
              onClick={() => void doInstall()}
              disabled={installDisabled}
            >
              {installing ? t("mcp.installing") : t("mcp.installSnippet")}
            </Button>
            {install && (
              <Button glyph={NF.copy} onClick={() => void copyIncludeLine()}>
                {t("mcp.copyInclude")}
              </Button>
            )}
          </div>
          {install && (
            // `.selectable` (base.css), not inline `userSelect: "text"` —
            // React omits the -webkit- prefix WKWebView reads first, so
            // the inline form never wins over the body opt-out.
            <div
              className="selectable"
              style={{
                padding: "var(--sp-10)",
                background: "var(--bg-sunken)",
                borderRadius: "var(--r-2)",
                fontSize: "var(--fs-2xs)",
                fontFamily: "var(--font-mono)",
              }}
            >
              <div>
                {t("mcp.wroteLine", { scope: install.scope, path: install.path })}
              </div>
              <div style={{ marginTop: "var(--sp-4)", fontWeight: 600 }}>{install.include_line}</div>
              {install.target_files.length > 0 && (
                <div style={{ marginTop: "var(--sp-6)", color: "var(--fg-muted)" }}>
                  {t("mcp.pasteInto")}
                  <ul style={{ margin: "var(--sp-4) 0 0 var(--sp-16)", padding: 0 }}>
                    {install.target_files.map((f) => (
                      <li key={f}>{f}</li>
                    ))}
                  </ul>
                </div>
              )}
            </div>
          )}
          <details style={{ marginTop: "var(--sp-4)" }}>
            <summary style={{ cursor: "pointer", fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}>
              {t("mcp.preview")}
            </summary>
            {/* <pre> is already select-opted-in by base.css; the old
                inline userSelect was redundant (and non-functional). */}
            <pre
              style={{
                marginTop: "var(--sp-8)",
                padding: "var(--sp-12)",
                background: "var(--bg-sunken)",
                borderRadius: "var(--r-2)",
                maxHeight: "var(--viewer-max-height)",
                overflow: "auto",
                fontSize: "var(--fs-2xs)",
                whiteSpace: "pre-wrap",
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
