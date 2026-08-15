import { useCallback, useEffect, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { ScreenHeader } from "../shell/ScreenHeader";
import { Button } from "../components/primitives/Button";
import { EmptyState as EmptyStatePrimitive } from "../components/primitives/EmptyState";
import { SkeletonList } from "../components/primitives/Skeleton";
import { NF } from "../icons";
import { api } from "../api";
import { renderError } from "../lib/i18n-error";
import { useAppState } from "../providers/AppStateProvider";
import type { PathStatus, RouteSettingsDto, RouteSummaryDto } from "../types";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { AddRouteModal, EditRouteModal } from "./third-party/AddRouteModal";
import { RouteCard } from "./third-party/RouteCard";
import {
  EVENT_OPEN_ADD_ROUTE,
  clearFromNetworkPanelBreadcrumb,
  consumeOpenAddRouteHint,
} from "../lib/networkPanelDeepLink";

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
  const { t } = useTranslation("providers");
  const { pushToast } = useAppState();
  const [routes, setRoutes] = useState<RouteSummaryDto[] | null>(null);
  const [settings, setSettings] = useState<RouteSettingsDto | null>(null);
  // Raw backend message, not the composed sentence — the "Load
  // failed: …" wrapper is applied at render time so a language
  // switch re-renders an error that is already on screen, and so
  // `refresh` stays free of a `t` dependency (it feeds a mount
  // effect; adding `t` would refetch on every language change).
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busyIds, setBusyIds] = useState<Set<string>>(new Set());
  // Cold-mount path: read the sessionStorage hint set by the
  // NetworkUnreachablePanel before this section mounted.
  const [showAdd, setShowAdd] = useState(() => consumeOpenAddRouteHint());
  const [editTarget, setEditTarget] = useState<RouteSummaryDto | null>(null);
  const [removeTarget, setRemoveTarget] = useState<RouteSummaryDto | null>(
    null,
  );
  const [restartHint, setRestartHint] = useState<
    "needed" | "applied" | "none"
  >("none");
  const [restartingDesktop, setRestartingDesktop] = useState(false);
  // Whether `~/.claudepot/bin` is on the shell PATH. Global state —
  // a wrapper that's been written is still unreachable until its
  // directory is on PATH. Probed by the login shell, not the GUI
  // process's own (minimal) env.
  const [pathStatus, setPathStatus] = useState<PathStatus>("unknown");
  const [addingToPath, setAddingToPath] = useState(false);

  const refresh = useCallback(async () => {
    // The shell PATH probe is supplementary and can be slow — keep it
    // off the critical path so it can neither fail nor stall the
    // whole section. Kicked off in parallel; its rejection collapses
    // to "unknown" instead of failing routes/settings.
    const pathStatusP = api
      .routesPathStatus()
      .catch((): PathStatus => "unknown");
    try {
      const [list, s] = await Promise.all([
        api.routesList(),
        api.routesSettingsGet(),
      ]);
      setRoutes(list);
      setSettings(s);
      setLoadError(null);
    } catch (e) {
      setLoadError(renderError(e));
    }
    setPathStatus(await pathStatusP);
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Hot-mount path for the NetworkUnreachablePanel's deep-link.
  // When this section is already mounted, `setSection("third-party")`
  // is a no-op and the cold-mount sessionStorage read won't re-fire,
  // so the panel's button needs a CustomEvent to reach us. See
  // `src/lib/networkPanelDeepLink.ts`.
  useEffect(() => {
    const handler = () => setShowAdd(true);
    window.addEventListener(EVENT_OPEN_ADD_ROUTE, handler);
    return () => window.removeEventListener(EVENT_OPEN_ADD_ROUTE, handler);
  }, []);

  const setBusy = (id: string, busy: boolean) => {
    setBusyIds((prev) => {
      const next = new Set(prev);
      if (busy) next.add(id);
      else next.delete(id);
      return next;
    });
  };

  const handleUseCli = async (id: string) => {
    setBusy(id, true);
    try {
      const r = await api.routesUseCli(id);
      await refresh();
      // The PATH banner (below) carries setup guidance when the
      // wrapper dir isn't reachable — keep the toast to the fact.
      pushToast(
        "info",
        t("section.wrapperInstalled", { name: r.wrapper_name }),
      );
    } catch (e) {
      pushToast("error", renderError(e, t("section.useCliFailed")));
    } finally {
      setBusy(id, false);
    }
  };

  const handleAddToPath = async () => {
    setAddingToPath(true);
    try {
      const rc = await api.routesAddToPath();
      pushToast("info", t("section.addedToPath", { rc }));
      // Re-probe to refresh the indicator. Best-effort: a probe
      // failure here does not undo the successful rc write, so it
      // must not surface as "Add to PATH failed".
      try {
        setPathStatus(await api.routesPathStatus());
      } catch {
        setPathStatus("unknown");
      }
    } catch (e) {
      pushToast("error", renderError(e, t("section.addToPathFailed")));
    } finally {
      setAddingToPath(false);
    }
  };

  const handleUnuseCli = async (id: string) => {
    setBusy(id, true);
    try {
      await api.routesUnuseCli(id);
      await refresh();
      pushToast("info", t("section.wrapperRemoved"));
    } catch (e) {
      pushToast("error", renderError(e, t("section.uninstallCliFailed")));
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
      pushToast("info", t("section.activeOnDesktopToast"));
    } catch (e) {
      pushToast("error", renderError(e, t("section.useDesktopFailed")));
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
      pushToast("info", t("section.desktopActivationCleared"));
    } catch (e) {
      pushToast("error", renderError(e, t("section.deactivateDesktopFailed")));
    } finally {
      setBusy(id, false);
    }
  };

  const handleRestartDesktop = async () => {
    setRestartingDesktop(true);
    try {
      await api.routesDesktopRestart();
      setRestartHint("applied");
      pushToast("info", t("section.desktopRestarted"));
    } catch (e) {
      pushToast("error", renderError(e, t("section.restartFailed")));
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
      pushToast("info", t("section.routeDeleted"));
    } catch (e) {
      pushToast("error", renderError(e, t("section.deleteFailed")));
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
      pushToast("error", renderError(e, t("section.settingsUpdateFailed")));
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
        title={t("section.title")}
        subtitle={t("section.subtitle")}
        actions={
          <Button
            variant="solid"
            glyph={NF.plus}
            onClick={() => setShowAdd(true)}
            title={t("section.addRouteTitle")}
          >
            {t("section.addRoute")}
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
        {loadError !== null && (
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
            {t("section.loadFailed", { message: loadError })}
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
            <span>{t("section.desktopRunningBanner")}</span>
            <Button
              variant="ghost"
              size="sm"
              onClick={handleRestartDesktop}
              disabled={restartingDesktop}
              glyph={NF.refresh}
            >
              {restartingDesktop
                ? t("section.restarting")
                : t("section.quitRelaunch")}
            </Button>
          </div>
        )}

        {pathStatus === "not_on_path" &&
          routes?.some((r) => r.installed_on_cli) && (
            <div
              role="status"
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                gap: "var(--sp-12)",
                padding: "var(--sp-10) var(--sp-14)",
                border: "var(--bw-hair) solid var(--warn)",
                background: "var(--warn-weak)",
                color: "var(--fg)",
                borderRadius: "var(--r-2)",
                fontSize: "var(--fs-sm)",
              }}
            >
              <span>
                <Trans
                  ns="providers"
                  i18nKey="section.notOnPathBanner"
                  components={{ code: <code /> }}
                />
              </span>
              <Button
                variant="ghost"
                size="sm"
                onClick={handleAddToPath}
                disabled={addingToPath}
                glyph={NF.terminal}
              >
                {addingToPath
                  ? t("section.addingToPath")
                  : t("section.addToPath")}
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
            title={t("section.hideChooserTitle")}
          >
            <input
              type="checkbox"
              checked={settings.disable_deployment_mode_chooser}
              onChange={toggleChooser}
            />
            {t("section.hideChooserLabel")}
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
                pathStatus={pathStatus}
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
        onClose={() => {
          setShowAdd(false);
          // Clear the network-panel breadcrumb so a future Add Route
          // (opened from the empty-state CTA, not from the network
          // panel) doesn't inherit the China-reachable highlight.
          clearFromNetworkPanelBreadcrumb();
        }}
        onCreated={() => {
          void refresh();
          pushToast("info", t("section.routeAdded"));
        }}
      />
      <EditRouteModal
        open={editTarget !== null}
        initialSummary={editTarget}
        onClose={() => setEditTarget(null)}
        onSaved={() => {
          void refresh();
          pushToast("info", t("section.routeUpdated"));
        }}
      />
      {removeTarget && (
        <ConfirmDialog
          title={t("removeDialog.title")}
          confirmLabel={t("removeDialog.confirm")}
          confirmDanger
          body={
            <p style={{ margin: 0, lineHeight: "var(--lh-body)" }}>
              <Trans
                ns="providers"
                i18nKey="removeDialog.body"
                values={{ name: removeTarget.name }}
                components={{ code: <code /> }}
              />
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
  const { t } = useTranslation("providers");
  return (
    <EmptyStatePrimitive
      // Left-aligned: the body is two paragraphs of explanation, not a
      // one-line "nothing here". Centering prose of this length reads
      // as a poster rather than an instruction.
      align="start"
      body={
        <>
          <p style={{ margin: 0 }}>{t("empty.intro")}</p>
          <p style={{ margin: "var(--sp-16) 0 0" }}>
            {/* The four wrapper names are examples, not data the user
                supplied — they stay inside the sentence so a translator
                can move them, and all four share one <code> mapping. */}
            <Trans
              ns="providers"
              i18nKey="empty.wrappers"
              components={{
                code: <code style={{ color: "var(--fg-strong)" }} />,
              }}
            />
          </p>
        </>
      }
      action={
        <Button variant="solid" glyph={NF.plus} onClick={onAdd}>
          {t("empty.addFirst")}
        </Button>
      }
    />
  );
}
