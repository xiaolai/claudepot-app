import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ScreenHeader } from "../shell/ScreenHeader";
import { Button } from "../components/primitives/Button";
import { SkeletonList } from "../components/primitives/Skeleton";
import { NF } from "../icons";
import { api } from "../api";
import type {
  AutomationSummaryDto,
  RouteSummaryDto,
  SchedulerCapabilitiesDto,
} from "../types";
import { ConfirmDialog } from "../components/ConfirmDialog";
import {
  AddAutomationModal,
  EditAutomationModal,
} from "./automations/AutomationModals";
import { AutomationCard } from "./automations/AutomationCard";
import { TemplateGallery } from "./templates/TemplateGallery";

/**
 * Automations section — define + run scheduled `claude -p` jobs.
 *
 * Mental model:
 * - Definitions live in `~/.claudepot/automations.json`.
 * - Each one materializes into the OS's native scheduler
 *   (launchd plist on macOS, systemd-user timer on Linux,
 *   Task Scheduler XML on Windows) plus a per-automation
 *   helper shim under `~/.claudepot/automations/<id>/run.sh`.
 * - "Run now" spawns the same shim out-of-band — distinct
 *   from scheduled runs which the OS scheduler invokes.
 *
 * v1: cron + manual triggers only. Reactive triggers
 * (fs-watch, webhook) are explicit v2.
 */
export function AutomationsSection() {
  const [automations, setAutomations] =
    useState<AutomationSummaryDto[] | null>(null);
  const [routes, setRoutes] = useState<RouteSummaryDto[]>([]);
  const [capabilities, setCapabilities] =
    useState<SchedulerCapabilitiesDto | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [toast, setToast] = useState<{
    kind: "info" | "error";
    msg: string;
  } | null>(null);
  const [busyIds, setBusyIds] = useState<Set<string>>(new Set());
  const [showAdd, setShowAdd] = useState(false);
  const [showGallery, setShowGallery] = useState(false);
  const [editTarget, setEditTarget] =
    useState<AutomationSummaryDto | null>(null);
  const [removeTarget, setRemoveTarget] =
    useState<AutomationSummaryDto | null>(null);
  const [runsRefreshKey, setRunsRefreshKey] = useState(0);

  const refresh = useCallback(async () => {
    try {
      const [list, rs, caps] = await Promise.all([
        api.automationsList(),
        api.routesList(),
        api.automationsSchedulerCapabilities(),
      ]);
      setAutomations(list);
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
      const opId = await api.automationsRunNowStart(id);
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
            setToast({
              kind: "error",
              msg: payload.detail ?? "Run failed.",
            });
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
      setToast({ kind: "error", msg: String(e) });
    }
  }

  async function handleToggle(id: string, enabled: boolean) {
    setBusy(id, true);
    try {
      await api.automationsSetEnabled(id, enabled);
      await refresh();
      setToast({
        kind: "info",
        msg: `Automation ${enabled ? "enabled" : "disabled"}.`,
      });
    } catch (e) {
      setToast({ kind: "error", msg: String(e) });
    } finally {
      setBusy(id, false);
    }
  }

  async function handleConfirmRemove() {
    if (!removeTarget) return;
    const id = removeTarget.id;
    setBusy(id, true);
    try {
      await api.automationsRemove(id);
      setRemoveTarget(null);
      await refresh();
      setToast({ kind: "info", msg: "Automation removed." });
    } catch (e) {
      setToast({ kind: "error", msg: String(e) });
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
        title="Automations"
        subtitle={`Scheduled and manual claude -p runs · ${
          capabilities?.native_label ?? "no scheduler"
        }`}
        actions={
          <>
            <Button
              variant="ghost"
              glyph={NF.refresh}
              onClick={refresh}
              disabled={automations === null}
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
            <Button
              variant="solid"
              glyph={NF.plus}
              onClick={() => setShowAdd(true)}
            >
              Add automation
            </Button>
          </>
        }
      />

      {loadError && (
        <div
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

      {toast && (
        <div
          style={{
            padding: "var(--sp-6) var(--sp-12)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-2)",
            color:
              toast.kind === "error" ? "var(--danger)" : "var(--fg-2)",
            fontSize: "var(--fs-sm)",
            background: "var(--bg-raised)",
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
          }}
        >
          <span>{toast.msg}</span>
          <Button variant="ghost" onClick={() => setToast(null)}>
            Dismiss
          </Button>
        </div>
      )}

      {automations === null ? (
        <SkeletonList rows={3} />
      ) : automations.length === 0 ? (
        <EmptyState onAdd={() => setShowAdd(true)} />
      ) : (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-8)",
          }}
        >
          {automations.map((a) => (
            <AutomationCard
              key={a.id}
              automation={a}
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

      <AddAutomationModal
        open={showAdd}
        routes={routes}
        capabilities={capabilities}
        onClose={() => setShowAdd(false)}
        onCreated={() => {
          refresh();
          setToast({ kind: "info", msg: "Automation created." });
        }}
        onError={(msg) => setToast({ kind: "error", msg })}
      />

      <TemplateGallery
        open={showGallery}
        onClose={() => setShowGallery(false)}
        onInstalled={() => {
          refresh();
          setToast({ kind: "info", msg: "Template installed." });
        }}
        onError={(msg) => setToast({ kind: "error", msg })}
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

      <EditAutomationModal
        open={!!editTarget}
        target={editTarget}
        routes={routes}
        capabilities={capabilities}
        onClose={() => setEditTarget(null)}
        onUpdated={() => {
          refresh();
          setToast({ kind: "info", msg: "Automation updated." });
        }}
        onError={(msg) => setToast({ kind: "error", msg })}
      />

      {removeTarget && (
        <ConfirmDialog
          title="Delete automation?"
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
        Add automation
      </Button>
    </div>
  );
}
