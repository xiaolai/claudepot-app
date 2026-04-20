import { lazy, Suspense, useCallback, useEffect, useMemo } from "react";
import { PendingJournalsBanner } from "./components/PendingJournalsBanner";
import { RunningOpStrip } from "./components/RunningOpStrip";
import { StatusIssuesBanner } from "./components/StatusIssuesBanner";
import { ToastContainer } from "./components/ToastContainer";
import { SplitBrainConfirm } from "./sections/accounts/SplitBrainConfirm";
import { sections, sectionIds } from "./sections/registry";
// Sections are code-split. The initial paint ships just the shell +
// AccountsSection (the default landing tab); Projects / Sessions /
// Settings + their heavy modal trees load on first navigation. Saves
// ~70 % of the JS that used to block first paint.
import { AccountsSection } from "./sections/AccountsSection";
// Named import promises so we can both hand them to React.lazy AND
// trigger them early for preload-on-mount (see `preloadSavedSection`
// below). Sharing the same factory ensures the bundler caches the
// module once — `React.lazy`'s first invocation and our preload call
// resolve to the same promise.
const importProjects = () =>
  import("./sections/ProjectsSection").then((m) => ({ default: m.ProjectsSection }));
const importSettings = () =>
  import("./sections/SettingsSection").then((m) => ({ default: m.SettingsSection }));
const importSessions = () =>
  import("./sections/SessionsSection").then((m) => ({ default: m.SessionsSection }));
const ProjectsSection = lazy(importProjects);
const SettingsSection = lazy(importSettings);
const SessionsSection = lazy(importSessions);
const OperationProgressModal = lazy(() =>
  import("./sections/projects/OperationProgressModal").then((m) => ({
    default: m.OperationProgressModal,
  })),
);

/** Kick off the saved section's chunk in parallel with first paint. */
function preloadSavedSection(): void {
  try {
    const id =
      localStorage.getItem("claudepot.startSection") ||
      localStorage.getItem("claudepot.activeSection");
    if (id === "projects") void importProjects();
    else if (id === "sessions") void importSessions();
    else if (id === "settings") void importSettings();
  } catch {
    // localStorage unavailable — nothing to preload.
  }
}
import { useSection } from "./hooks/useSection";
import { usePendingJournals } from "./hooks/usePendingJournals";
import { useRunningOps } from "./hooks/useRunningOps";
import { bindingFrom } from "./hooks/useAccounts";
import { useStatusIssues } from "./hooks/useStatusIssues";
import { useTheme } from "./hooks/useTheme";
import { OperationsProvider, useOperations } from "./hooks/useOperations";
import { AppStateProvider, useAppState } from "./providers/AppStateProvider";
import { api } from "./api";
import { listen } from "@tauri-apps/api/event";
import type { RunningOpInfo } from "./types";
import { WindowChrome, AppSidebar, AppStatusBar } from "./shell";

function AppShell() {
  const { section, subRoute, setSection, setSubRoute } = useSection(
    sectionIds[0],
    sectionIds,
  );
  const { summary: pendingSummary, refresh: refreshPendingBanner } =
    usePendingJournals();
  const { ops: runningOps } = useRunningOps();
  const { active: activeOp, open: openOp, close: closeOp } = useOperations();
  const {
    accounts,
    status: appStatus,
    ccIdentity,
    syncError,
    keychainIssue,
    refresh: refreshAccounts,
    toasts,
    dismissToast,
    pushToast,
    isDismissed,
    dismiss,
    actions,
    requestCliSwap,
    splitBrainPending,
    dismissSplitBrain,
    confirmSplitBrain,
  } = useAppState();
  const { resolved: themeResolved, toggle: toggleTheme } = useTheme();

  // Binding derived from the same source of truth AccountsSection
  // uses — the active flags on each account. When the user binds via
  // the sidebar switcher, we call cli_use / desktop_use on the
  // backend and then re-fetch to pick up the new flags.
  const binding = useMemo(() => bindingFrom(accounts), [accounts]);

  const labelFor = (op: RunningOpInfo): string => {
    const verb =
      op.kind === "repair_resume"
        ? "Resuming"
        : op.kind === "repair_rollback"
          ? "Rolling back"
          : "Renaming";
    const base = (p: string) => p.split("/").filter(Boolean).pop() ?? p;
    return `${verb} ${base(op.old_path)} → ${base(op.new_path)}`;
  };

  // Kick off the saved section's lazy chunk the moment the shell
  // mounts, so the import is in flight (or already cached) by the
  // time useSection's idle callback swaps to it. Runs once per mount.
  useEffect(() => {
    preloadSavedSection();
  }, []);

  // Cmd+, opens Settings (standard macOS shortcut)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (
        (e.metaKey || e.ctrlKey) &&
        e.key === "," &&
        !e.shiftKey &&
        !e.altKey
      ) {
        e.preventDefault();
        setSection("settings");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setSection]);

  // App menu bar + tray menu both emit `app-menu` with a string id as
  // payload. Routing lives here (not in the section) because nav items
  // need the shell-level setSection. Action items delegate to the
  // section via window events to avoid entangling state trees.
  useEffect(() => {
    const unlistenPromise = listen<string>("app-menu", (event) => {
      const cmd = event.payload;
      if (cmd.startsWith("app-menu:nav:")) {
        const sub = cmd.substring("app-menu:nav:".length).split(":")[0];
        if (sectionIds.includes(sub)) setSection(sub);
        return;
      }
      if (cmd === "app-menu:view:toggle-theme") {
        toggleTheme();
        return;
      }
      if (cmd === "app-menu:view:reload") {
        void refreshAccounts();
        return;
      }
      if (cmd === "app-menu:account:login-browser") {
        setSection("accounts");
        window.dispatchEvent(new CustomEvent("cp-open-add"));
        return;
      }
      if (cmd === "app-menu:account:sync-cc") {
        api
          .syncFromCurrentCc()
          .then((email) =>
            pushToast(
              "info",
              email ? `Synced ${email} from CC.` : "Nothing to sync.",
            ),
          )
          .catch((e) => pushToast("error", `Sync failed: ${e}`));
        return;
      }
      if (cmd === "app-menu:account:verify-all") {
        api
          .verifyAllAccounts()
          .then(() => {
            pushToast("info", "Verify all complete.");
            void refreshAccounts();
          })
          .catch((e) => pushToast("error", `Verify failed: ${e}`));
        return;
      }
      if (cmd === "app-menu:help:copy-diag") {
        setSection("settings");
        pushToast("info", "Open Settings → Diagnostics and press Copy.");
        return;
      }
    });
    return () => {
      void unlistenPromise.then((f) => f());
    };
  }, [setSection, toggleTheme, refreshAccounts, pushToast]);

  const onRepairSubview =
    section === "projects" &&
    (subRoute === "repair" || subRoute === "maintenance");
  const actionableTotal =
    pendingSummary === null
      ? 0
      : pendingSummary.pending + pendingSummary.stale;
  const showBanner =
    pendingSummary !== null && actionableTotal > 0 && !onRepairSubview;

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

  // Jump to Accounts and pass the UUID so the section can scroll to
  // (or highlight) the flagged row. Deep-link plumbing is currently a
  // section jump — AccountsSection subscribes to this event and owns
  // the row-level focus.
  const onSelectAccount = useCallback(
    (uuid: string) => {
      setSection("accounts");
      window.dispatchEvent(
        new CustomEvent("cp-focus-account", { detail: uuid }),
      );
    },
    [setSection],
  );

  const onUnlockKeychain = useCallback(async () => {
    try {
      await api.unlockKeychain();
      await refreshAccounts();
    } catch (e) {
      pushToast("error", `Unlock failed: ${e}`);
    }
  }, [pushToast, refreshAccounts]);

  const onReloginActive = useCallback(() => {
    const active = accounts.find((a) => a.is_cli_active);
    if (!active) {
      pushToast("error", "No active CLI account to re-login.");
      return;
    }
    // Shared login helper owns the busy keyring, cancel affordance,
    // and tray refresh — no reason to re-implement it inline.
    void actions.login(active);
  }, [accounts, actions, pushToast]);

  const rawIssues = useStatusIssues({
    ccIdentity,
    status: appStatus,
    syncError,
    keychainIssue,
    accounts,
    onUnlock: onUnlockKeychain,
    onSelectAccount,
    onReloginActive,
  });
  const visibleIssues = useMemo(
    () => rawIssues.filter((i) => !(i.dismissable && isDismissed(i.id))),
    [rawIssues, isDismissed],
  );

  // Breadcrumb tail mirrors the active section.
  const cwd = useMemo(
    () => sections.find((s) => s.id === section)?.label.toLowerCase() ?? section,
    [section],
  );

  const openPalette = useCallback(() => {
    // The command palette currently lives inside AccountsSection (it
    // owns the account-level actions). As a minimal bridge, dispatch
    // a window event that AccountsSection already listens for.
    window.dispatchEvent(new CustomEvent("cp-open-palette"));
  }, []);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100vh",
        background: "var(--bg)",
        color: "var(--fg)",
        fontFamily: "var(--font)",
        fontSize: "var(--fs-base)",
      }}
    >
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
          }}
          version="v0.4.2"
          synced
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

          {showBanner && pendingSummary && (
            <div style={{ padding: "var(--sp-12) var(--sp-16) 0" }}>
              <PendingJournalsBanner
                summary={pendingSummary}
                onOpen={() => setSection("projects", "repair")}
              />
            </div>
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
              {section === "accounts" && (
                <AccountsSection onNavigate={setSection} />
              )}
              {section === "projects" && (
                <ProjectsSection
                  subRoute={subRoute}
                  onSubRouteChange={setSubRoute}
                />
              )}
              {section === "sessions" && <SessionsSection />}
              {section === "settings" && <SettingsSection />}
            </Suspense>
          </div>

          <RunningOpStrip
            ops={runningOps}
            onReopen={(opId) => {
              const op = runningOps.find((o) => o.op_id === opId);
              if (!op) return;
              openOp({
                opId,
                title: labelFor(op),
              });
            }}
          />
        </main>
      </div>

      <AppStatusBar
        stats={{
          branch: "main",
          projects: null,
          sessions: null,
          model: "claude-sonnet-4-5",
        }}
      />

      {activeOp && (
        <Suspense fallback={null}>
          <OperationProgressModal
            key={activeOp.opId}
            opId={activeOp.opId}
            title={activeOp.title}
            onClose={closeOp}
            onComplete={() => {
              activeOp.onComplete?.();
              refreshPendingBanner();
            }}
            onError={(detail) => {
              activeOp.onError?.(detail);
              refreshPendingBanner();
            }}
            onOpenRepair={() => {
              closeOp();
              setSection("projects", "repair");
            }}
          />
        </Suspense>
      )}

      {splitBrainPending && (
        <SplitBrainConfirm
          account={splitBrainPending}
          onCancel={dismissSplitBrain}
          onConfirm={confirmSplitBrain}
        />
      )}

      <ToastContainer toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
}

function App() {
  return (
    <OperationsProvider>
      <AppStateProvider>
        <AppShell />
      </AppStateProvider>
    </OperationsProvider>
  );
}

export default App;
