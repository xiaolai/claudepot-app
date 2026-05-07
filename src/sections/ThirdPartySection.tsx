import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ScreenHeader } from "../shell/ScreenHeader";
import { Button } from "../components/primitives/Button";
import { SkeletonList } from "../components/primitives/Skeleton";
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
  const { t } = useTranslation();
  const [routes, setRoutes] = useState<RouteSummaryDto[] | null>(null);
  const [settings, setSettings] = useState<RouteSettingsDto | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [toast, setToast] = useState<{
    kind: "info" | "error";
    msg: string;
  } | null>(null);
  const [busyIds, setBusyIds] = useState<Set<string>>(new Set());
  // Track the auto-dismiss timer so a fast unmount or a second toast
  // doesn't fire setToast on a dead component / replace the timer with
  // a stale one. `clearTimeout` on undefined is a no-op.
  const toastTimerRef = useRef<number | undefined>(undefined);
  useEffect(() => {
    return () => {
      if (toastTimerRef.current !== undefined) {
        window.clearTimeout(toastTimerRef.current);
      }
    };
  }, []);
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
    if (toastTimerRef.current !== undefined) {
      window.clearTimeout(toastTimerRef.current);
    }
    toastTimerRef.current = window.setTimeout(() => {
      setToast(null);
      toastTimerRef.current = undefined;
    }, 4500);
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
          ? t("thirdParty.wrapperInstalled", { name: r.wrapper_name })
          : t("thirdParty.wrapperRemoved"),
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
      showToast("info", t("thirdParty.wrapperRemoved"));
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
      showToast("info", t("thirdParty.activeOnDesktop"));
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
      showToast("info", t("thirdParty.desktopCleared"));
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
      showToast("info", t("thirdParty.desktopRestarted"));
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
      showToast("info", t("thirdParty.routeDeleted"));
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
        title={t("thirdParty.title")}
        subtitle={t("thirdParty.subtitle")}
        actions={
          <Button
            variant="solid"
            glyph={NF.plus}
            onClick={() => setShowAdd(true)}
            title={t("thirdParty.addRouteTitle")}
          >
            {t("thirdParty.addRoute")}
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
              {t("thirdParty.restartBanner")}
            </span>
            <Button
              variant="solid"
              size="sm"
              onClick={handleRestartDesktop}
              disabled={restartingDesktop}
              glyph={NF.refresh}
            >
              {restartingDesktop
                ? t("thirdParty.restarting")
                : t("thirdParty.restartButton")}
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
            title={t("thirdParty.hideChooser")}
          >
            <input
              type="checkbox"
              checked={settings.disable_deployment_mode_chooser}
              onChange={toggleChooser}
            />
            {t("thirdParty.hideChooser")}
          </label>
        )}

        {routes === null ? (
          <SkeletonList rows={3} />
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
          showToast("info", t("thirdParty.routeAdded"));
        }}
        onError={(msg) => showToast("error", msg)}
      />
      <EditRouteModal
        open={editTarget !== null}
        initialSummary={editTarget}
        onClose={() => setEditTarget(null)}
        onSaved={() => {
          void refresh();
          showToast("info", t("thirdParty.routeUpdated"));
        }}
        onError={(msg) => showToast("error", msg)}
      />
      {removeTarget && (
        <ConfirmDialog
          title={t("thirdParty.deleteTitle")}
          confirmLabel={t("thirdParty.deleteConfirm")}
          confirmDanger
          body={
            <p style={{ margin: 0, lineHeight: "var(--lh-body)" }}>
              {t("thirdParty.deleteBody", { name: removeTarget.name })}
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
  const { t } = useTranslation();
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
        lineHeight: "var(--lh-loose)",
      }}
    >
      <p style={{ margin: 0 }}>
        {t("thirdParty.emptyText1")}
      </p>
      <p style={{ margin: 0 }}>
        {t("thirdParty.emptyText2")}
      </p>
      <Button variant="solid" glyph={NF.plus} onClick={onAdd}>
        {t("thirdParty.addFirstRoute")}
      </Button>
    </div>
  );
}
