import { useCallback, useEffect, useState } from "react";
import { ScreenHeader } from "../shell/ScreenHeader";
import { Button } from "../components/primitives/Button";
import { NF } from "../icons";
import { api } from "../api";
import type { RouteSettingsDto, RouteSummaryDto } from "../types";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { AddRouteModal, EditRouteModal } from "./third-party/AddRouteModal";
import { RouteCard } from "./third-party/RouteCard";

/**
 * Third-party section — entry point for non-Anthropic LLM routes.
 *
 * Phase 2. Full design in `dev-docs/third-party-llm-design.md`.
 *
 * Mental model:
 *   - First-party `claude` CLI keeps reading from the
 *     `Claude Code-credentials` keychain entry — never touched.
 *   - First-party Claude Desktop keeps reading from
 *     `~/Library/Application Support/Claude/` — never touched.
 *   - Third-party routes live in their own dimension: each one
 *     installs as a separate wrapper binary on PATH
 *     (`~/.claudepot/bin/<name>`) and as a Desktop profile in
 *     `~/Library/Application Support/Claude-3p/`.
 */
export function ThirdPartySection() {
  const [routes, setRoutes] = useState<RouteSummaryDto[] | null>(null);
  const [settings, setSettings] = useState<RouteSettingsDto | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [toast, setToast] = useState<{
    kind: "info" | "error";
    msg: string;
  } | null>(null);
  const [busyIds, setBusyIds] = useState<Set<string>>(new Set());
  const [showAdd, setShowAdd] = useState(false);
  const [editTarget, setEditTarget] = useState<RouteSummaryDto | null>(null);
  const [removeTarget, setRemoveTarget] = useState<RouteSummaryDto | null>(
    null,
  );
  const [restartHint, setRestartHint] = useState<
    "needed" | "applied" | "none"
  >("none");
  const [restartingDesktop, setRestartingDesktop] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [list, s] = await Promise.all([
        api.routesList(),
        api.routesSettingsGet(),
      ]);
      setRoutes(list);
      setSettings(s);
      setLoadError(null);
    } catch (e) {
      setLoadError(`Load failed: ${e instanceof Error ? e.message : e}`);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const setBusy = (id: string, busy: boolean) => {
    setBusyIds((prev) => {
      const next = new Set(prev);
      if (busy) next.add(id);
      else next.delete(id);
      return next;
    });
  };

  const showToast = (kind: "info" | "error", msg: string) => {
    setToast({ kind, msg });
    window.setTimeout(() => setToast(null), 4500);
  };

  const handleUseCli = async (id: string) => {
    setBusy(id, true);
    try {
      await api.routesUseCli(id);
      await refresh();
      const r = (await api.routesList()).find((x) => x.id === id);
      showToast(
        "info",
        r
          ? `Wrapper installed: \`${r.wrapper_name}\`. Add ~/.claudepot/bin to PATH if you haven't already.`
          : "Wrapper installed.",
      );
    } catch (e) {
      showToast("error", `Use in CLI failed: ${e instanceof Error ? e.message : e}`);
    } finally {
      setBusy(id, false);
    }
  };

  const handleUnuseCli = async (id: string) => {
    setBusy(id, true);
    try {
      await api.routesUnuseCli(id);
      await refresh();
      showToast("info", "Wrapper removed.");
    } catch (e) {
      showToast("error", `Uninstall CLI failed: ${e instanceof Error ? e.message : e}`);
    } finally {
      setBusy(id, false);
    }
  };

  const flagRestartIfRunning = async () => {
    try {
      if (await api.routesDesktopRunning()) {
        setRestartHint("needed");
      } else {
        setRestartHint("none");
      }
    } catch {
      // Probe failure is non-fatal; default to showing the banner so
      // the user is reminded to restart if Desktop is in fact open.
      setRestartHint("needed");
    }
  };

  const handleUseDesktop = async (id: string) => {
    setBusy(id, true);
    try {
      await api.routesUseDesktop(id);
      await refresh();
      await flagRestartIfRunning();
      showToast("info", "Active on Desktop.");
    } catch (e) {
      showToast("error", `Use in Desktop failed: ${e instanceof Error ? e.message : e}`);
    } finally {
      setBusy(id, false);
    }
  };

  const handleUnuseDesktop = async (id: string) => {
    setBusy(id, true);
    try {
      await api.routesUnuseDesktop();
      await refresh();
      await flagRestartIfRunning();
      showToast("info", "Desktop activation cleared.");
    } catch (e) {
      showToast("error", `Deactivate Desktop failed: ${e instanceof Error ? e.message : e}`);
    } finally {
      setBusy(id, false);
    }
  };

  const handleRestartDesktop = async () => {
    setRestartingDesktop(true);
    try {
      await api.routesDesktopRestart();
      setRestartHint("applied");
      showToast("info", "Claude Desktop restarted.");
    } catch (e) {
      showToast(
        "error",
        `Restart failed: ${e instanceof Error ? e.message : e}`,
      );
    } finally {
      setRestartingDesktop(false);
    }
  };

  const handleRemove = (id: string) => {
    const target = routes?.find((r) => r.id === id) ?? null;
    if (target) setRemoveTarget(target);
  };

  const executeRemove = async (route: RouteSummaryDto) => {
    setBusy(route.id, true);
    try {
      await api.routesRemove(route.id);
      await refresh();
      if (route.active_on_desktop) {
        await flagRestartIfRunning();
      }
      showToast("info", "Route deleted.");
    } catch (e) {
      showToast("error", `Delete failed: ${e instanceof Error ? e.message : e}`);
    } finally {
      setBusy(route.id, false);
    }
  };

  const toggleChooser = async () => {
    if (!settings) return;
    try {
      const next = await api.routesSettingsSet({
        disable_deployment_mode_chooser: !settings.disable_deployment_mode_chooser,
      });
      setSettings(next);
    } catch (e) {
      showToast("error", `Settings update failed: ${e instanceof Error ? e.message : e}`);
    }
  };

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        overflow: "hidden",
      }}
    >
      <ScreenHeader
        title="Third-parties"
        subtitle="Run Claude Code and Claude Desktop with non-Anthropic LLMs"
        actions={
          <Button
            variant="solid"
            glyph={NF.plus}
            onClick={() => setShowAdd(true)}
            title="Configure a new third-party route"
          >
            Add route
          </Button>
        }
      />

      <div
        style={{
          flex: 1,
          overflow: "auto",
          padding: "var(--sp-24) var(--sp-32)",
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-20)",
        }}
      >
        {loadError && (
          <div
            role="alert"
            style={{
              padding: "var(--sp-12) var(--sp-16)",
              border: "var(--bw-hair) solid var(--danger-border, var(--line))",
              borderRadius: "var(--r-2)",
              color: "var(--fg)",
              fontSize: "var(--fs-sm)",
            }}
          >
            {loadError}
          </div>
        )}

        {toast && (
          <div
            role={toast.kind === "error" ? "alert" : "status"}
            style={{
              padding: "var(--sp-10) var(--sp-14)",
              borderRadius: "var(--r-2)",
              border: "var(--bw-hair) solid var(--line)",
              background: "var(--bg-raised)",
              color: toast.kind === "error" ? "var(--danger-fg, var(--fg))" : "var(--fg)",
              fontSize: "var(--fs-sm)",
            }}
          >
            {toast.msg}
          </div>
        )}

        {restartHint === "needed" && (
          <div
            role="status"
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              gap: "var(--sp-12)",
              padding: "var(--sp-10) var(--sp-14)",
              border: "var(--bw-hair) solid var(--accent-border)",
              background: "var(--accent-soft)",
              color: "var(--accent-ink)",
              borderRadius: "var(--r-2)",
              fontSize: "var(--fs-sm)",
            }}
          >
            <span>
              Claude Desktop is running. Restart it to apply the new
              configuration.
            </span>
            <Button
              variant="solid"
              size="sm"
              onClick={handleRestartDesktop}
              disabled={restartingDesktop}
              glyph={NF.refresh}
            >
              {restartingDesktop
                ? "Restarting…"
                : "Quit & relaunch Claude Desktop"}
            </Button>
          </div>
        )}

        {settings && (
          <label
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-8)",
              fontSize: "var(--fs-sm)",
              color: "var(--fg-faint)",
            }}
            title="When enabled, Claude Desktop skips the launch-time chooser and commits to the active mode."
          >
            <input
              type="checkbox"
              checked={settings.disable_deployment_mode_chooser}
              onChange={toggleChooser}
            />
            Hide the deployment-mode chooser at Claude Desktop launch
          </label>
        )}

        {routes === null ? (
          <p style={{ color: "var(--fg-faint)" }}>Loading…</p>
        ) : routes.length === 0 ? (
          <EmptyState onAdd={() => setShowAdd(true)} />
        ) : (
          <div
            style={{
              display: "grid",
              gridTemplateColumns:
                "repeat(auto-fill, minmax(var(--content-cap-sm), 1fr))",
              gap: "var(--sp-16)",
            }}
          >
            {routes.map((r) => (
              <RouteCard
                key={r.id}
                route={r}
                busy={busyIds.has(r.id)}
                onUseCli={handleUseCli}
                onUnuseCli={handleUnuseCli}
                onUseDesktop={handleUseDesktop}
                onUnuseDesktop={handleUnuseDesktop}
                onRemove={handleRemove}
                onEdit={(route) => setEditTarget(route)}
              />
            ))}
          </div>
        )}
      </div>

      <AddRouteModal
        open={showAdd}
        onClose={() => setShowAdd(false)}
        onCreated={() => {
          void refresh();
          showToast("info", "Route added.");
        }}
        onError={(msg) => showToast("error", msg)}
      />
      <EditRouteModal
        open={editTarget !== null}
        initialSummary={editTarget}
        onClose={() => setEditTarget(null)}
        onSaved={() => {
          void refresh();
          showToast("info", "Route updated.");
        }}
        onError={(msg) => showToast("error", msg)}
      />
      {removeTarget && (
        <ConfirmDialog
          title="Delete route?"
          confirmLabel="Delete route"
          confirmDanger
          body={
            <p style={{ margin: 0, lineHeight: 1.5 }}>
              <code>{removeTarget.name}</code>'s CLI wrapper will be
              removed and its Desktop activation cleared. The route
              definition cannot be recovered without recreating it.
            </p>
          }
          onCancel={() => setRemoveTarget(null)}
          onConfirm={() => {
            const target = removeTarget;
            setRemoveTarget(null);
            void executeRemove(target);
          }}
        />
      )}
    </div>
  );
}

function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "flex-start",
        gap: "var(--sp-16)",
        padding: "var(--sp-32)",
        maxWidth: 720,
        color: "var(--fg)",
        fontSize: "var(--fs-sm)",
        lineHeight: 1.6,
      }}
    >
      <p style={{ margin: 0 }}>
        No routes yet. Add a route to run Claude Code or Claude Desktop
        against a non-Anthropic backend — Bedrock, Vertex, Foundry, or
        any Anthropic-Messages-compatible gateway (Ollama, vLLM,
        OpenRouter, Kimi, DeepSeek, GLM, LiteLLM, …).
      </p>
      <p style={{ margin: 0 }}>
        Each route installs a wrapper command on PATH —{" "}
        <code style={{ color: "var(--fg-strong)" }}>claude-llama3</code>,{" "}
        <code style={{ color: "var(--fg-strong)" }}>claude-kimi</code>,{" "}
        <code style={{ color: "var(--fg-strong)" }}>
          claude-bedrock-prod
        </code>{" "}
        — and (optionally) a profile in Claude Desktop&rsquo;s native
        configuration registry. The first-party{" "}
        <code style={{ color: "var(--fg-strong)" }}>claude</code> binary
        and your Anthropic account are never touched.
      </p>
      <Button variant="solid" glyph={NF.plus} onClick={onAdd}>
        Add your first route
      </Button>
    </div>
  );
}
