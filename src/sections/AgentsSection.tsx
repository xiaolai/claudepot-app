import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ScreenHeader } from "../shell/ScreenHeader";
import { Button } from "../components/primitives/Button";
import { SkeletonList } from "../components/primitives/Skeleton";
import { NF } from "../icons";
import { api } from "../api";
import { useAppState } from "../providers/AppStateProvider";
import type {
  AgentSummaryDto,
  RouteSummaryDto,
  SchedulerCapabilitiesDto,
} from "../types";
import { ConfirmDialog } from "../components/ConfirmDialog";
import {
  AddAgentModal,
  EditAgentModal,
} from "./agents/AgentModals";
import { AgentCard } from "./agents/AgentCard";
import { TemplateGallery } from "./templates/TemplateGallery";

/**
 * Agents section — define + run scheduled `claude -p` jobs.
 *
 * Mental model:
 * - Definitions live in `~/.claudepot/agents.json`.
 * - Each one materializes into the OS's native scheduler
 *   (launchd plist on macOS, systemd-user timer on Linux,
 *   Task Scheduler XML on Windows) plus a per-agent
 *   helper shim under `~/.claudepot/agents/<id>/run.sh`.
 * - "Run now" spawns the same shim out-of-band — distinct
 *   from scheduled runs which the OS scheduler invokes.
 *
 * v1: cron + manual triggers only. Reactive triggers
 * (fs-watch, webhook) are explicit v2.
 */
export function AgentsSection() {
  const { pushToast } = useAppState();
  const [agents, setAgents] =
    useState<AgentSummaryDto[] | null>(null);
  const [routes, setRoutes] = useState<RouteSummaryDto[]>([]);
  const [capabilities, setCapabilities] =
    useState<SchedulerCapabilitiesDto | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busyIds, setBusyIds] = useState<Set<string>>(new Set());
  const [showAdd, setShowAdd] = useState(false);
  const [showGallery, setShowGallery] = useState(false);
  const [editTarget, setEditTarget] =
    useState<AgentSummaryDto | null>(null);
  const [removeTarget, setRemoveTarget] =
    useState<AgentSummaryDto | null>(null);
  const [runsRefreshKey, setRunsRefreshKey] = useState(0);

  const refresh = useCallback(async () => {
    try {
      const [list, rs, caps] = await Promise.all([
        api.agentsList(),
        api.routesList(),
        api.agentsSchedulerCapabilities(),
      ]);
      setAgents(list);
      setRoutes(rs);
      setCapabilities(caps);
      setLoadError(null);
    } catch (e) {
      setLoadError(String(e));
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  function setBusy(id: string, on: boolean) {
    setBusyIds((prev) => {
      const next = new Set(prev);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  }

  async function handleRun(id: string) {
    setBusy(id, true);
    let unlisten: (() => void) | null = null;
    let timeoutHandle: number | undefined;
    const cleanup = () => {
      if (unlisten) {
        try {
          unlisten();
        } catch {
          /* listener already torn down */
        }
        unlisten = null;
      }
      if (timeoutHandle !== undefined) {
        window.clearTimeout(timeoutHandle);
        timeoutHandle = undefined;
      }
    };
    try {
      const opId = await api.agentsRunNowStart(id);
      // Listen for the terminal event on this op channel. The
      // backend (src-tauri/src/ops.rs::ProgressEvent) emits the
      // terminal event as `{phase: "op", status: "complete" | "error", ...}`.
      // Per-phase events use other phase names with status="running"
      // / "complete" — we only fire on the op-level signal.
      unlisten = await listen<{
        phase: string;
        status: string;
        detail?: string;
      }>(`op-progress::${opId}`, (event) => {
        const payload = event.payload;
        if (payload.phase === "op") {
          if (payload.status === "error") {
            pushToast("error", payload.detail ?? "Run failed.");
          } else {
            setRunsRefreshKey((k) => k + 1);
          }
          setBusy(id, false);
          cleanup();
        }
      });
      // Safety timeout in case the event channel drops — clear busy
      // after 5 minutes so the UI doesn't get stuck forever.
      timeoutHandle = window.setTimeout(() => {
        setBusy(id, false);
        cleanup();
      }, 5 * 60 * 1000);
    } catch (e) {
      cleanup();
      setBusy(id, false);
      pushToast("error", String(e));
    }
  }

  async function handleToggle(id: string, enabled: boolean) {
    setBusy(id, true);
    try {
      await api.agentsSetEnabled(id, enabled);
      await refresh();
      pushToast(
        "info",
        `Agent ${enabled ? "enabled" : "disabled"}.`,
      );
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setBusy(id, false);
    }
  }

  async function handleConfirmRemove() {
    if (!removeTarget) return;
    const id = removeTarget.id;
    setBusy(id, true);
    try {
      await api.agentsRemove(id);
      setRemoveTarget(null);
      await refresh();
      pushToast("info", "Agent removed.");
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setBusy(id, false);
    }
  }

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-12)",
        padding: "var(--sp-16)",
      }}
    >
      <ScreenHeader
        title="Agents"
        subtitle={`Scheduled and manual claude -p runs · ${
          capabilities?.native_label ?? "no scheduler"
        }`}
        actions={
          <>
            <Button
              variant="ghost"
              glyph={NF.refresh}
              onClick={refresh}
              disabled={agents === null}
            >
              Refresh
            </Button>
            <Button
              variant="ghost"
              glyph={NF.copy}
              onClick={() => setShowGallery(true)}
            >
              From template…
            </Button>
            {agents !== null && agents.length > 0 && (
              <Button
                variant="solid"
                glyph={NF.plus}
                onClick={() => setShowAdd(true)}
              >
                Add agent
              </Button>
            )}
          </>
        }
      />

      {loadError && (
        <div
          role="alert"
          style={{
            padding: "var(--sp-8) var(--sp-12)",
            border: "var(--bw-hair) solid var(--danger)",
            borderRadius: "var(--r-2)",
            color: "var(--danger)",
            fontSize: "var(--fs-sm)",
          }}
        >
          {loadError}
        </div>
      )}

      {agents === null ? (
        <SkeletonList rows={3} />
      ) : agents.length === 0 ? (
        <EmptyState onAdd={() => setShowAdd(true)} />
      ) : (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-8)",
          }}
        >
          {agents.map((a) => (
            <AgentCard
              key={a.id}
              agent={a}
              busy={busyIds.has(a.id)}
              runsRefreshKey={runsRefreshKey}
              onRun={handleRun}
              onEdit={setEditTarget}
              onToggle={handleToggle}
              onRemove={setRemoveTarget}
            />
          ))}
        </div>
      )}

      <AddAgentModal
        open={showAdd}
        routes={routes}
        capabilities={capabilities}
        onClose={() => setShowAdd(false)}
        onCreated={() => {
          refresh();
          pushToast("info", "Agent created.");
        }}
      />

      <TemplateGallery
        open={showGallery}
        onClose={() => setShowGallery(false)}
        onInstalled={() => {
          refresh();
          pushToast("info", "Template installed.");
        }}
        onOpenThirdParties={() => {
          // Best-effort deep-link: dispatch a custom event the
          // sidebar/router listens to. If nothing handles it,
          // close the gallery so the user can navigate manually.
          window.dispatchEvent(new CustomEvent("claudepot:nav", {
            detail: { section: "third-parties" },
          }));
          setShowGallery(false);
        }}
      />

      <EditAgentModal
        open={!!editTarget}
        target={editTarget}
        routes={routes}
        capabilities={capabilities}
        onClose={() => setEditTarget(null)}
        onUpdated={() => {
          refresh();
          pushToast("info", "Agent updated.");
        }}
      />

      {removeTarget && (
        <ConfirmDialog
          title="Delete agent?"
          body={`'${removeTarget.display_name || removeTarget.name}' will be unregistered from the OS scheduler and its run history removed.`}
          confirmLabel="Delete"
          confirmDanger
          onConfirm={handleConfirmRemove}
          onCancel={() => setRemoveTarget(null)}
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
        alignItems: "center",
        gap: "var(--sp-12)",
        padding: "var(--sp-32) var(--sp-16)",
        border: "var(--bw-hair) dashed var(--line)",
        borderRadius: "var(--r-3)",
        color: "var(--fg-2)",
        textAlign: "center",
      }}
    >
      <h3 style={{ margin: 0, fontSize: "var(--fs-md)", color: "var(--fg)" }}>
        Schedule a claude -p run
      </h3>
      <p style={{ margin: 0, fontSize: "var(--fs-sm)", maxWidth: "60ch" }}>
        Project commands and agents in the chosen folder are picked up
        automatically. Use a slash-command for the prompt to keep
        complex jobs versioned in your repo.
      </p>
      <Button variant="solid" glyph={NF.plus} onClick={onAdd}>
        Add agent
      </Button>
    </div>
  );
}
