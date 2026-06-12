import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { StatusIssuesBanner } from "./components/StatusIssuesBanner";
import { NetworkUnreachablePanel } from "./components/NetworkUnreachablePanel";
import { useNetworkGate } from "./hooks/useNetworkGate";
import {
  triggerOpenAddRoute,
  triggerSettingsTab,
} from "./lib/networkPanelDeepLink";
import { ToastContainer } from "./components/ToastContainer";
import { ShellCommandPalette } from "./components/ShellCommandPalette";
import { QuitConfirm } from "./components/QuitConfirm";
import { ShortcutsModal } from "./components/ShortcutsModal";
import { AccountConfirmModals } from "./components/AccountConfirmModals";
import { NotificationBridges } from "./components/NotificationBridges";
import { OperationProgressHost } from "./components/OperationProgressHost";
import { ConsentLiveModal } from "./components/ConsentLiveModal";
// Sections are code-split. The registry owns the chunk factories,
// the lazy components, and the preload helpers — App.tsx only
// composes. The initial paint ships just the shell + AccountsSection
// (the default landing tab, eager in the registry); everything else
// loads on first navigation or via the idle preload below.
import {
  sections,
  sectionIds,
  preloadSavedSection,
  preloadAllSections,
  type SectionHostProps,
} from "./sections/registry";
import { ErrorBoundary } from "./ErrorBoundary";
import {
  SESSION_MOVE_PHASES,
  renderSessionMoveResult,
} from "./sections/projects/sessionMoveProgress";
import { useSection } from "./hooks/useSection";
import { usePendingJournals } from "./hooks/usePendingJournals";
import { useRunningOps } from "./hooks/useRunningOps";
import { bindingFrom } from "./hooks/useAccounts";
import { useShellStatusIssues } from "./hooks/useShellStatusIssues";
import { useTheme } from "./hooks/useTheme";
import { useSidebarCollapsed } from "./hooks/useSidebarCollapsed";
import { OperationsProvider, useOperations } from "./hooks/useOperations";
import { AppStateProvider, useAppState } from "./providers/AppStateProvider";
import { UpdateProvider } from "./providers/UpdateProvider";
import { useActivityAlertCount } from "./hooks/useActivityNotifications";
import { useActivityConsentGate } from "./hooks/useActivityConsentGate";
import { useAppMenuRouter } from "./hooks/useAppMenuRouter";
import { useNavigationBridges } from "./hooks/useNavigationBridges";
import { useNotificationClickRouter } from "./hooks/useNotificationClickRouter";
import { useShellShortcuts } from "./hooks/useShellShortcuts";
import { useTrafficLightSync } from "./hooks/useTrafficLightSync";
import { useTrayBridge } from "./hooks/useTrayBridge";
import { api } from "./api";
import type { LiveSessionSummary, RunningOpInfo } from "./types";
import { WindowChrome, AppSidebar, AppStatusBar } from "./shell";
import { APP_VERSION } from "./version";

/** Verb label for a running op in the status bar / progress modal. */
function labelFor(op: RunningOpInfo): string {
  const verb =
    op.kind === "repair_resume"
      ? "Resuming"
      : op.kind === "repair_rollback"
        ? "Rolling back"
        : op.kind === "session_move"
          ? "Moving session"
          : "Renaming";
  const base = (p: string) => p.split("/").filter(Boolean).pop() ?? p;
  return `${verb} ${base(op.old_path)} → ${base(op.new_path)}`;
}

/**
 * Composition root for the paper-mono shell. Cross-cutting behavior
 * lives in per-concern hooks (tray bridge, app-menu router,
 * notification-click router, shell shortcuts, desktop identity
 * sync, …) and leaf components (NotificationBridges,
 * AccountConfirmModals, OperationProgressHost) — AppShell only owns
 * the state those concerns share and the layout that renders them.
 */
function AppShell() {
  const { section, subRoute, setSection, setSubRoute } = useSection(
    sectionIds[0],
    sectionIds,
  );
  /**
   * Transcript file to open when ProjectsSection next mounts. Written
   * by the cross-section command-palette bridge, the Activity-surface
   * card click, and the live-session jump from the dashboard strip;
   * cleared by ProjectsSection's pending-consumer effect after it
   * resolves the matching project and seeds `openedSessionPath`.
   * This replaces an earlier setTimeout(0) dispatch that could drop
   * the selection if the lazy section hadn't finished mounting.
   */
  const [pendingSessionPath, setPendingSessionPath] = useState<string | null>(
    null,
  );
  /**
   * Project cwd to pre-select when the Projects section next mounts.
   * Set when a card in the Activity surface fires
   * `claudepot:navigate-section` with `projectPath`. Consumed by
   * ProjectsSection alongside `pendingSessionPath`.
   */
  const [pendingProjectPath, setPendingProjectPath] = useState<string | null>(
    null,
  );
  // Palette + shortcuts modal live at shell level so ⌘K / ⌘/ open
  // without forcing a section switch.
  const [showPalette, setShowPalette] = useState(false);
  const [showShortcuts, setShowShortcuts] = useState(false);

  // First-run gate for the live Activity feature; also starts the
  // live runtime when consent was already granted.
  const consentGate = useActivityConsentGate();
  const { summary: pendingSummary, refresh: refreshPendingBanner } =
    usePendingJournals();
  const { ops: runningOps } = useRunningOps();
  // First-run network reachability gate. Probes api.anthropic.com
  // once on mount; renders the unreachable panel below the
  // StatusIssuesBanner when the probe fails AND the user hasn't
  // dismissed for this session. See
  // `dev-docs/network-detection-panel.md`.
  const networkGate = useNetworkGate();
  const { open: openOp } = useOperations();
  const {
    accounts,
    refresh: refreshAccounts,
    toasts,
    dismissToast,
    pushToast,
    emit,
    actions,
    requestCliSwap,
    requestDesktopSignOut,
  } = useAppState();
  const { resolved: themeResolved, toggle: toggleTheme } = useTheme();
  const { collapsed: sidebarCollapsed, toggle: toggleSidebar } =
    useSidebarCollapsed();

  // Binding derived from the same source of truth AccountsSection
  // uses — the active flags on each account. When the user binds via
  // the sidebar switcher, we call cli_use / desktop_use on the
  // backend and then re-fetch to pick up the new flags.
  const binding = useMemo(() => bindingFrom(accounts), [accounts]);

  // Kick off the saved section's lazy chunk the moment the shell
  // mounts, so the import is in flight (or already cached) by the
  // time useSection's idle callback swaps to it. Then trickle every
  // remaining section chunk into the module cache during idle time
  // so that later in-app navigation never blocks on a chunk fetch
  // (which used to surface as a blank Suspense-fallback flash on
  // every first visit to a new section). Runs once per mount.
  useEffect(() => {
    preloadSavedSection();
    preloadAllSections();
  }, []);

  // Pin the WindowChrome breadcrumb / ⌘K pill onto the OS-placed
  // traffic lights' actual centerline.
  useTrafficLightSync();

  const openPalette = useCallback(() => setShowPalette(true), []);
  const openShortcuts = useCallback(() => setShowShortcuts(true), []);

  // ⌘, / ⌘K / ⌘/ / ⌃⌥⌘L / ⌘⇧L. (⌘1..⌘9 lives in useSection.)
  useShellShortcuts({ setSection, openPalette, openShortcuts, pushToast });

  // Window-event bridges: `claudepot:navigate-section` (cross-section
  // deep links) + `cp-goto-session` (palette session search).
  useNavigationBridges({
    setSection,
    setPendingSessionPath,
    setPendingProjectPath,
  });

  // App menu bar + tray menu `app-menu` command routing.
  useAppMenuRouter({ setSection, toggleTheme, refreshAccounts, pushToast });

  // OS-notification click routing via the focus-event heuristic +
  // the bell popover's stored-target round-trip.
  useNotificationClickRouter({
    setSection,
    setPendingSessionPath,
    setPendingProjectPath,
    accounts,
  });

  // Alerting-session count (errored / stuck / waiting) for the
  // Activity nav badge + tray mirror. Subscribes through a
  // primitive snapshot, so the shell re-renders only when the COUNT
  // changes — not on every live-list publish (the transition-diffing
  // OS-notification work lives in NotificationBridges' leaf).
  const activityAlerts = useActivityAlertCount();

  // Tray → main-window bridge: alert-count mirror, Activity row
  // click, Desktop clear/bind, CLI switch feedback (+ Undo toast).
  useTrayBridge({
    alertCount: activityAlerts,
    setSection,
    setPendingSessionPath,
    setPendingProjectPath,
    requestDesktopSignOut,
    accounts,
    actions,
    pushToast,
    emit,
    refreshAccounts,
  });

  // Suppress the pending-journals chip while the user is already on
  // the surface that resolves it — duplicating the call-to-action a
  // foot below the Repair view itself reads as nagging.
  const onRepairSubview =
    section === "projects" &&
    (subRoute === "repair" || subRoute === "maintenance");

  const handleBind = useCallback(
    async (target: "cli" | "desktop", uuid: string) => {
      const account = accounts.find((a) => a.uuid === uuid);
      if (!account) return;
      // Route through the shared action helpers so sidebar binds pick
      // up the same split-brain preflight, busy-keyring, toast queue,
      // and tray-refresh that the Accounts page uses.
      if (target === "cli") {
        await requestCliSwap(account);
      } else {
        await actions.useDesktop(account, true);
      }
    },
    [accounts, actions, requestCliSwap],
  );

  // Status-issues pipeline: banner actions + Desktop identity sync +
  // issue derivation + snooze filtering/auto-clear + banner emits.
  const { visibleIssues, dismiss } = useShellStatusIssues(setSection);

  // Registry entry for the active section — drives both the breadcrumb
  // tail and the section render switch.
  const activeSection = useMemo(
    () => sections.find((s) => s.id === section),
    [section],
  );

  // Breadcrumb tail mirrors the active section.
  const cwd = activeSection?.label.toLowerCase() ?? section;

  /** Shell-owned state handed to the registry's per-section render
   *  functions. Each section picks the subset it accepts. */
  const sectionHostProps: SectionHostProps = {
    subRoute,
    onSubRouteChange: setSubRoute,
    onNavigate: setSection,
    pendingProjectPath,
    pendingSessionPath,
    onPendingConsumed: () => {
      setPendingProjectPath(null);
      setPendingSessionPath(null);
    },
  };

  const openLiveSession = useCallback(
    (s: LiveSessionSummary) => {
      if (s.transcript_path) {
        setPendingSessionPath(s.transcript_path);
        if (s.cwd) setPendingProjectPath(s.cwd);
        // Sessions land inside Projects after the events-into-
        // projects collapse; ProjectsSection's pending consumer
        // selects the matching project and opens the transcript.
        setSection("projects");
      }
    },
    [setSection],
  );

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "var(--viewport-h-full)",
        background: "var(--bg)",
        color: "var(--fg)",
        fontFamily: "var(--font)",
        fontSize: "var(--fs-base)",
      }}
    >
      {/* Ambient notification hooks live in this null-rendering leaf
          so their internal live-session subscription re-renders only
          the leaf, never the shell tree. */}
      <NotificationBridges />

      <WindowChrome
        cwd={cwd}
        theme={themeResolved}
        onToggleTheme={toggleTheme}
        onCmdK={openPalette}
      />

      <div
        style={{
          flex: 1,
          display: "flex",
          minHeight: 0,
          overflow: "hidden",
        }}
      >
        <AppSidebar
          sections={sections}
          active={section}
          onSelect={(id) => setSection(id)}
          accounts={accounts}
          binding={binding}
          onBind={handleBind}
          badges={{
            accounts: accounts.length || undefined,
            // Alerting sessions (errored / stuck / waiting) surface
            // as the badge count on the Activity nav row — the merged
            // dashboard + events surface that now owns "what's
            // happening right now."
            events: activityAlerts || undefined,
          }}
          version={APP_VERSION}
          synced
          data-sidebar-root
          onOpenLiveSession={openLiveSession}
          collapsed={sidebarCollapsed}
          onToggleCollapsed={toggleSidebar}
        />

        <main
          style={{
            flex: 1,
            minWidth: 0,
            display: "flex",
            flexDirection: "column",
            overflow: "hidden",
            position: "relative",
          }}
        >
          <StatusIssuesBanner issues={visibleIssues} onDismiss={dismiss} />

          {/* First-run network detection panel. See
              `dev-docs/network-detection-panel.md`. Renders only when
              api.anthropic.com is unreachable AND the user hasn't
              dismissed it for this session. */}
          {networkGate.shouldShowPanel && networkGate.state.kind === "unreachable" && (
            <NetworkUnreachablePanel
              diagnosis={networkGate.state.diagnosis}
              onRetry={networkGate.retry}
              onDismiss={networkGate.dismiss}
              onUseProvider={() => {
                // triggerOpenAddRoute sets sessionStorage (cold-mount
                // path) AND dispatches a CustomEvent (hot-mount path
                // for when ThirdPartySection is already mounted).
                // See `src/lib/networkPanelDeepLink.ts`.
                triggerOpenAddRoute();
                setSection("third-party");
              }}
              onConfigureProxy={() => {
                triggerSettingsTab("network");
                setSection("settings");
              }}
            />
          )}

          <div
            style={{
              flex: 1,
              minHeight: 0,
              display: "flex",
              flexDirection: "column",
              overflow: "hidden",
            }}
          >
            <Suspense fallback={null}>
              {/* Per-section ErrorBoundary keys on the section id so a
                  prior error state resets cleanly when the user
                  navigates away — no stale "Try again" panel after a
                  tab switch. Both the body and the boundary label come
                  from the registry — the single source of truth for
                  section wiring. */}
              {activeSection && (
                <ErrorBoundary
                  key={activeSection.id}
                  label={activeSection.label}
                >
                  {activeSection.render(sectionHostProps)}
                </ErrorBoundary>
              )}
            </Suspense>
          </div>
        </main>
      </div>

      <AppStatusBar
        stats={{
          projects: null,
          sessions: null,
        }}
        runningOps={runningOps}
        onReopenOp={(opId: string) => {
          const op = runningOps.find((o) => o.op_id === opId);
          if (!op) return;
          if (op.kind === "session_move") {
            openOp({
              opId,
              title: labelFor(op),
              phases: SESSION_MOVE_PHASES,
              fetchStatus: api.sessionMoveStatus,
              renderResult: renderSessionMoveResult,
            });
            return;
          }
          openOp({
            opId,
            title: labelFor(op),
          });
        }}
        pendingSummary={onRepairSubview ? null : pendingSummary}
        onOpenRepair={() => setSection("projects", "repair")}
        onOpenLive={() => setSection("events")}
        sidebarCollapsed={sidebarCollapsed}
        onToggleSidebar={toggleSidebar}
      />

      {/* Data-driven overlays get their own scoped boundaries so a
          render crash in one of them can't take down the shell via
          main.tsx's full-takeover boundary. */}
      <OperationProgressHost
        onTerminal={refreshPendingBanner}
        onOpenRepair={() => setSection("projects", "repair")}
      />

      <AccountConfirmModals />

      <ShellCommandPalette
        open={showPalette}
        onClose={() => setShowPalette(false)}
        onNavigate={setSection}
        onShowShortcuts={openShortcuts}
      />

      {showShortcuts && (
        <ShortcutsModal onClose={() => setShowShortcuts(false)} />
      )}

      <ToastContainer toasts={toasts} onDismiss={dismissToast} />

      {/* First-run gate for the live Activity feature. Re-opens
          only if the backend prefs report consent still hasn't
          been seen. */}
      <ConsentLiveModal open={consentGate.open} onDismiss={consentGate.dismiss} />

      {/* Quit-gate modal. Self-contained — listens on
          `cp-quit-requested` and is a no-op until the Rust side
          decides ⌘Q (or tray Quit) needs confirmation because
          `RunningOps` has live entries. */}
      <QuitConfirm />
    </div>
  );
}

function App() {
  return (
    <OperationsProvider>
      <AppStateProvider>
        <UpdateProvider>
          <AppShell />
        </UpdateProvider>
      </AppStateProvider>
    </OperationsProvider>
  );
}

export default App;
