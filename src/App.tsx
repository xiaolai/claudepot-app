import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { PendingJournalsBanner } from "./components/PendingJournalsBanner";
import { RunningOpStrip } from "./components/RunningOpStrip";
import { StatusIssuesBanner } from "./components/StatusIssuesBanner";
import { ToastContainer } from "./components/ToastContainer";
import { CommandPalette } from "./components/CommandPalette";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { ShortcutsModal } from "./components/ShortcutsModal";
import { SplitBrainConfirm } from "./sections/accounts/SplitBrainConfirm";
import { DesktopConfirmDialog } from "./sections/accounts/DesktopConfirmDialog";
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
const importActivities = () =>
  import("./sections/ActivitiesSection").then((m) => ({
    default: m.ActivitiesSection,
  }));
const importEvents = () =>
  import("./sections/EventsSection").then((m) => ({ default: m.EventsSection }));
const importTrends = () =>
  import("./sections/TrendsSection").then((m) => ({ default: m.TrendsSection }));
const importKeys = () =>
  import("./sections/KeysSection").then((m) => ({ default: m.KeysSection }));
const importConfig = () =>
  import("./sections/ConfigSection").then((m) => ({ default: m.ConfigSection }));
const importGlobal = () =>
  import("./sections/GlobalSection").then((m) => ({ default: m.GlobalSection }));
const ProjectsSection = lazy(importProjects);
const SettingsSection = lazy(importSettings);
const ActivitiesSection = lazy(importActivities);
const EventsSection = lazy(importEvents);
const TrendsSection = lazy(importTrends);
// SessionsSection is mounted transitively through ActivitiesSection
// now; keep the lazy factory around so its chunk is cached by the
// prefetcher without us needing a second export here.
void importSessions;
const KeysSection = lazy(importKeys);
const GlobalSection = lazy(importGlobal);
// ConfigSection isn't rendered at the top level anymore — it lives
// inside GlobalSection and the Projects shell's Config tab. The
// import* chunk keys off GlobalSection's own import, and
// ProjectsSection statically imports ConfigSection, so we don't need
// to warm it separately. Keep the factory reference so tree-shaking
// can't drop the export accidentally.
void importConfig;
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
    else if (id === "activities") void importActivities();
    else if (id === "events") void importEvents();
    else if (id === "trends") void importTrends();
    else if (id === "global") void importGlobal();
    else if (id === "keys") void importKeys();
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
import { ConsentLiveModal } from "./components/ConsentLiveModal";
import { useActivityNotifications } from "./hooks/useActivityNotifications";
import { useCardNotifications } from "./sections/events/useCardNotifications";
import { listen } from "@tauri-apps/api/event";
import type { LiveSessionSummary, RunningOpInfo } from "./types";
import { WindowChrome, AppSidebar, AppStatusBar } from "./shell";
import { APP_VERSION } from "./version";

function AppShell() {
  const { section, subRoute, setSection, setSubRoute } = useSection(
    sectionIds[0],
    sectionIds,
  );
  /**
   * Path to select when the Sessions tab next mounts. Written by the
   * cross-session command-palette bridge; cleared by SessionsSection
   * the first time it mounts with a pending value. This replaces an
   * earlier setTimeout(0) dispatch that could drop the selection if
   * the lazy section hadn't finished mounting.
   */
  const [pendingSessionPath, setPendingSessionPath] = useState<string | null>(
    null,
  );
  // Palette + shortcuts modal live at shell level so ⌘K / ⌘/ open
  // without forcing a section switch.
  const [showPalette, setShowPalette] = useState(false);
  const [showShortcuts, setShowShortcuts] = useState(false);

  // Live Activity consent. On cold launch we ask the backend once
  // whether the consent modal still needs to fire. `true` = modal
  // open; `false` = either already accepted/declined, or the prefs
  // fetch failed (fail-closed: no modal means no surprise reads).
  const [showConsentModal, setShowConsentModal] = useState(false);
  // On cold launch: fire the consent modal if the user has never been
  // asked, otherwise start the live runtime when activity is enabled.
  // The former sidebar "Off" chip (for the dedicated Activity row)
  // went away with the C-1 A consolidation — Sessions' Live filter
  // covers the same "is the runtime on" signal indirectly.
  useEffect(() => {
    let cancelled = false;
    api
      .preferencesGet()
      .then((p) => {
        if (cancelled) return;
        if (!p.activity_consent_seen) setShowConsentModal(true);
        else if (p.activity_enabled) {
          api.sessionLiveStart().catch(() => {});
        }
      })
      .catch(() => {
        // Prefs fetch failed (non-Tauri env). Leave modal closed.
      });
    return () => {
      cancelled = true;
    };
  }, []);
  const { summary: pendingSummary, refresh: refreshPendingBanner } =
    usePendingJournals();
  const { ops: runningOps } = useRunningOps();
  const { active: activeOp, open: openOp, close: closeOp } = useOperations();
  const {
    accounts,
    status: appStatus,
    ccIdentity,
    syncError,
    authRejectedAt,
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
    desktopConfirmPending,
    requestDesktopSignOut,
    requestDesktopOverwrite,
    dismissDesktopConfirm,
    confirmDesktopPending,
    removeConfirmPending,
    requestRemoveAccount,
    dismissRemoveConfirm,
    confirmRemoveAccount,
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

  // Shell-level keyboard shortcuts: ⌘, opens Settings, ⌘K opens the
  // palette, ⌘/ opens the shortcuts reference. All three skip editable
  // focus so typing inside an input doesn't hijack them.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod || e.altKey) return;
      const el = document.activeElement as HTMLElement | null;
      const tag = el?.tagName?.toLowerCase();
      const editable =
        tag === "input" || tag === "textarea" || el?.isContentEditable;

      if (e.key === "," && !e.shiftKey) {
        e.preventDefault();
        setSection("settings");
        return;
      }
      if ((e.key === "k" || e.key === "K") && !e.shiftKey) {
        if (editable) return;
        e.preventDefault();
        setShowPalette(true);
        return;
      }
      if (e.key === "/" && !e.shiftKey) {
        if (editable) return;
        e.preventDefault();
        setShowShortcuts(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setSection]);

  // Cross-section navigation requests via DOM CustomEvent. Lets a
  // child section (e.g. EventsSection card click) switch to another
  // section AND seed the target session without prop-drilling
  // `setSection` or coupling component trees. Payload shape:
  // `{ id: string, sessionPath?: string }`.
  //
  // When `sessionPath` is set, also seeds `pendingSessionPath` so
  // the destination Sessions section opens the right transcript on
  // mount. Per-line scroll-to-byte-offset is Phase 6 — landing on
  // the right session is the MVP.
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<{ id?: string; sessionPath?: string }>)
        .detail;
      const id = detail?.id;
      if (id && sectionIds.includes(id)) {
        if (detail.sessionPath) {
          setPendingSessionPath(detail.sessionPath);
        }
        setSection(id);
      }
    };
    window.addEventListener("claudepot:navigate-section", handler);
    return () =>
      window.removeEventListener("claudepot:navigate-section", handler);
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

  // Bridge from the command palette's cross-session search: when the
  // user selects a session hit, stash the target path and jump to the
  // Sessions tab. `SessionsSection` reads the pending path on mount —
  // so the selection is guaranteed to be consumed even if the section
  // wasn't already rendered. Previous implementation raced via
  // `setTimeout(0)` and could drop the selection on slow mounts.
  useEffect(() => {
    function onGoto(ev: Event) {
      const detail = (ev as CustomEvent<{ filePath: string }>).detail;
      if (!detail?.filePath) return;
      setPendingSessionPath(detail.filePath);
      setSection("activities");
    }
    window.addEventListener("cp-goto-session", onGoto);
    return () => window.removeEventListener("cp-goto-session", onGoto);
  }, [setSection]);

  // Activity notifications — fires OS notifications for user-enabled
  // trigger classes (error burst, idle-after-work, stuck). Returns the
  // count of alerting sessions (errored/stuck) for the nav badge,
  // reusing the hook's internal useSessionLive subscription.
  const activityAlerts = useActivityNotifications();
  // Card-level OS notifications. Fires on Warn+ CardEmitted deltas
  // gated by `notify_on_error`. Coalesces same-title bursts (≥3 in
  // 60 s → one summary notification). See
  // src/sections/events/useCardNotifications.ts for the rules.
  useCardNotifications();

  // Tray → Activity row click lands on the Tauri event
  // `cp-activity-open-session` with the session id as payload.
  // Resolve to a transcript path via the live runtime's snapshot so
  // the existing Sessions deep-link pipe handles routing. If the
  // session isn't in the snapshot (already ended between click and
  // handler), just switch to Sessions.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen<string>("cp-activity-open-session", async (ev) => {
      const sid = ev.payload;
      if (!sid) return;
      try {
        const snap = await api.sessionLiveSnapshot();
        const row = snap.find((s) => s.session_id === sid);
        if (row?.transcript_path) {
          setPendingSessionPath(row.transcript_path);
        }
      } catch {
        /* fallback to just switching */
      }
      setSection("activities");
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {
        /* no-tauri env */
      });
    return () => {
      unlisten?.();
    };
  }, [setSection]);

  // Tray Desktop actions route through the shell's confirmation
  // modal: the tray itself can't render a modal, so it emits events
  // the main window converts into the same DesktopConfirmDialog
  // flow as the in-window context menu + palette.
  useEffect(() => {
    let unlistenClear: (() => void) | null = null;
    let unlistenBind: (() => void) | null = null;
    listen("cp-tray-desktop-clear", () => requestDesktopSignOut())
      .then((fn) => {
        unlistenClear = fn;
      })
      .catch(() => {});
    listen("cp-tray-desktop-bind", () => {
      // Route to Accounts so the adoption banner / context menu is
      // visible — the user picks a target account there.
      setSection("accounts");
    })
      .then((fn) => {
        unlistenBind = fn;
      })
      .catch(() => {});
    return () => {
      unlistenClear?.();
      unlistenBind?.();
    };
  }, [requestDesktopSignOut, setSection]);

  // ⌘⇧L — focus the first SidebarLiveStrip row. Light-weight
  // fallback until the Activity section lands (M4) and claims this
  // shortcut. Ignores editable focus so typing "L" in the command
  // palette isn't hijacked.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod || !e.shiftKey || e.altKey) return;
      if (e.key !== "l" && e.key !== "L") return;
      const el = document.activeElement as HTMLElement | null;
      const tag = el?.tagName?.toLowerCase();
      if (tag === "input" || tag === "textarea" || el?.isContentEditable) {
        return;
      }
      e.preventDefault();
      // The strip renders with role=listbox; focus the first option.
      const firstRow = document.querySelector<HTMLButtonElement>(
        '[aria-label="Live Claude sessions"] [role="option"]',
      );
      firstRow?.focus();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

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

  // Adopt CC's currently-authenticated login as a new Claudepot
  // account. Surfaced by the CC-slot-drift banner when the drifted
  // email isn't already registered — saves a Sign-out → Add → OAuth
  // round-trip because the credential already exists.
  const onImportCurrent = useCallback(
    async (email: string) => {
      try {
        const outcome = await api.accountAddFromCurrent();
        pushToast("info", `Imported ${outcome.email}`);
        await refreshAccounts();
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        pushToast("error", `Import failed: ${msg}`);
      }
      // `email` is intentionally unused — supplied by the hook so the
      // button label can show the address, but the backend reads CC's
      // current state directly.
      void email;
    },
    [pushToast, refreshAccounts],
  );

  // Run the live Desktop identity sync on shell mount AND on window
  // focus. Each probe costs one Keychain read + one /profile HTTP
  // call (~1s) — hammering every Alt-Tab would be unfriendly, so
  // the cadence is throttled by a last-run timestamp ref mirroring
  // useRefresh's VERIFY_TTL pattern. Default 5-minute cooldown is
  // long enough that routine window-focus noise doesn't trigger,
  // but short enough that leaving Claudepot open while signing into
  // Desktop elsewhere catches the change within one focus cycle.
  const [desktopSync, setDesktopSync] = useState<
    import("./types").DesktopSyncOutcome | null
  >(null);
  const desktopSyncLastRun = useRef<number>(0);
  const DESKTOP_SYNC_TTL_MS = 5 * 60_000;

  const runDesktopSync = useCallback(
    async (force: boolean) => {
      const now = Date.now();
      if (!force && now - desktopSyncLastRun.current < DESKTOP_SYNC_TTL_MS) return;
      desktopSyncLastRun.current = now;
      try {
        const outcome = await api.syncFromCurrentDesktop();
        setDesktopSync(outcome);
        // Verified means the backend may have pointed `active_desktop`
        // at a different account (see sync_from_current). The
        // `is_desktop_active` flags in our accounts list are now stale
        // — refresh so badges match truth without waiting for the next
        // unrelated refresh to happen to win the race.
        if (outcome.kind === "verified") {
          await refreshAccounts();
        }
      } catch {
        // Slow-path failure (keychain locked, /profile down) is not a
        // user-surfaceable error here — the banner layer already shows
        // CandidateOnly when it can. Swallow.
      }
    },
    [DESKTOP_SYNC_TTL_MS, refreshAccounts],
  );

  useEffect(() => {
    void runDesktopSync(true); // cold-start probe runs unthrottled
  }, [runDesktopSync]);

  useEffect(() => {
    const onFocus = () => void runDesktopSync(false);
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [runDesktopSync]);

  const onAdoptLiveDesktop = useCallback(
    (email: string) => {
      const match = accounts.find(
        (a) => a.email.toLowerCase() === email.toLowerCase(),
      );
      if (!match) {
        pushToast("error", `No registered account matches ${email}.`);
        return;
      }
      // Banner action never implicitly overwrites — if the match
      // already has a snapshot, route through the shell-level
      // confirm modal so the user opts in to replacing it.
      if (match.desktop_profile_on_disk) requestDesktopOverwrite(match);
      else void actions.adoptDesktop(match);
    },
    [accounts, actions, pushToast, requestDesktopOverwrite],
  );

  const onImportDesktop = useCallback(
    (email: string) => {
      // Same as CC-side Import: jump to Accounts + open Add modal.
      // The user completes browser login there, then triggers adopt.
      setSection("accounts");
      window.dispatchEvent(new CustomEvent("cp-open-add"));
      void email; // label-only; the modal reads live state itself.
    },
    [setSection],
  );

  const rawIssues = useStatusIssues({
    ccIdentity,
    status: appStatus,
    syncError,
    authRejectedAt,
    keychainIssue,
    accounts,
    onUnlock: onUnlockKeychain,
    onSelectAccount,
    onReloginActive,
    onImportCurrent,
    desktopSync,
    onAdoptLiveDesktop,
    onImportDesktop,
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

  const openPalette = useCallback(() => setShowPalette(true), []);

  const openLiveSession = useCallback(
    (s: LiveSessionSummary) => {
      if (s.transcript_path) {
        setPendingSessionPath(s.transcript_path);
        setSection("activities");
      }
    },
    [setSection],
  );

  // "Open in Config" used to cross-section hop to the standalone
  // Config tab. With the restructure, Config is a tab inside the
  // Projects shell, so the button simply selects the project + flips
  // the tab locally (handled inside ProjectsSection).

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
            // Alerting sessions (errored / stuck) surface as the
            // badge count on the Activities nav row.
            activities: activityAlerts || undefined,
          }}
          version={APP_VERSION}
          synced
          data-sidebar-root
          onOpenLiveSession={openLiveSession}
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
              {section === "activities" && (
                <ActivitiesSection
                  initialSelectedPath={pendingSessionPath}
                  onInitialSelectedPathConsumed={() =>
                    setPendingSessionPath(null)
                  }
                />
              )}
              {section === "events" && <EventsSection />}
              {section === "trends" && <TrendsSection />}
              {section === "global" && (
                <GlobalSection
                  subRoute={subRoute}
                  onSubRouteChange={setSubRoute}
                />
              )}
              {section === "keys" && <KeysSection />}
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
          projects: null,
          sessions: null,
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

      {desktopConfirmPending && (
        <DesktopConfirmDialog
          request={desktopConfirmPending}
          onCancel={dismissDesktopConfirm}
          onConfirm={confirmDesktopPending}
        />
      )}

      {removeConfirmPending && (
        <ConfirmDialog
          title="Remove account?"
          confirmLabel="Remove"
          confirmDanger
          body={
            <>
              <p>
                Remove <strong>{removeConfirmPending.email}</strong>?
              </p>
              <p className="muted small">
                Deletes credentials and Desktop profile. Active
                CLI/Desktop pointers will be cleared. You'll have a few
                seconds to undo from the toast.
              </p>
            </>
          }
          onCancel={dismissRemoveConfirm}
          onConfirm={confirmRemoveAccount}
        />
      )}

      {showPalette && appStatus && (
        <CommandPalette
          accounts={accounts}
          status={appStatus}
          onClose={() => setShowPalette(false)}
          onSwitchCli={(a) => void requestCliSwap(a)}
          onSwitchDesktop={(a) => void actions.useDesktop(a)}
          onAdd={() => {
            setSection("accounts");
            window.dispatchEvent(new CustomEvent("cp-open-add"));
          }}
          onRefresh={() => void refreshAccounts()}
          onRemove={(a) => requestRemoveAccount(a)}
          onAdoptDesktop={(a) => {
            if (a.desktop_profile_on_disk) requestDesktopOverwrite(a);
            else void actions.adoptDesktop(a);
          }}
          onClearDesktop={requestDesktopSignOut}
          onLaunchDesktop={() => {
            api.desktopLaunch().catch((e) => {
              const msg = e instanceof Error ? e.message : String(e);
              pushToast("error", `Desktop launch failed: ${msg}`);
            });
          }}
          onNavigate={setSection}
          onShowShortcuts={() => setShowShortcuts(true)}
        />
      )}

      {showShortcuts && (
        <ShortcutsModal onClose={() => setShowShortcuts(false)} />
      )}

      <ToastContainer toasts={toasts} onDismiss={dismissToast} />

      {/* First-run gate for the live Activity feature. Re-opens
          only if the backend prefs report consent still hasn't
          been seen. */}
      <ConsentLiveModal
        open={showConsentModal}
        onDismiss={() => setShowConsentModal(false)}
      />
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
