import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { StatusIssuesBanner } from "./components/StatusIssuesBanner";
import { ToastContainer } from "./components/ToastContainer";
import { CommandPalette } from "./components/CommandPalette";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { QuitConfirm } from "./components/QuitConfirm";
import { ShortcutsModal } from "./components/ShortcutsModal";
import { SplitBrainConfirm } from "./sections/accounts/SplitBrainConfirm";
import { DesktopConfirmDialog } from "./sections/accounts/DesktopConfirmDialog";
import { sections, sectionIds } from "./sections/registry";
import { ErrorBoundary } from "./ErrorBoundary";
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
const importEvents = () =>
  import("./sections/EventsSection").then((m) => ({ default: m.EventsSection }));
const importKeys = () =>
  import("./sections/KeysSection").then((m) => ({ default: m.KeysSection }));
const importConfig = () =>
  import("./sections/ConfigSection").then((m) => ({ default: m.ConfigSection }));
const importGlobal = () =>
  import("./sections/GlobalSection").then((m) => ({ default: m.GlobalSection }));
const importThirdParty = () =>
  import("./sections/ThirdPartySection").then((m) => ({ default: m.ThirdPartySection }));
const importAutomations = () =>
  import("./sections/AutomationsSection").then((m) => ({ default: m.AutomationsSection }));
const ProjectsSection = lazy(importProjects);
const SettingsSection = lazy(importSettings);
const EventsSection = lazy(importEvents);
const KeysSection = lazy(importKeys);
const GlobalSection = lazy(importGlobal);
const ThirdPartySection = lazy(importThirdParty);
const AutomationsSection = lazy(importAutomations);
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
import {
  PROJECT_MOVE_PHASES,
  renderProjectMoveResult,
} from "./sections/projects/projectMoveProgress";
import {
  SESSION_MOVE_PHASES,
  renderSessionMoveResult,
} from "./sections/projects/sessionMoveProgress";

/** Kick off the saved section's chunk in parallel with first paint. */
function preloadSavedSection(): void {
  try {
    const id =
      localStorage.getItem("claudepot.startSection") ||
      localStorage.getItem("claudepot.activeSection");
    if (id === "projects") void importProjects();
    else if (id === "events") void importEvents();
    else if (id === "global") void importGlobal();
    else if (id === "keys") void importKeys();
    else if (id === "third-party") void importThirdParty();
    else if (id === "automations") void importAutomations();
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
import { UpdateProvider } from "./providers/UpdateProvider";
import { readDevMode, writeDevMode } from "./hooks/useDevMode";
import { api } from "./api";
import { toastError } from "./lib/toastError";
import { ConsentLiveModal } from "./components/ConsentLiveModal";
import { useActivityNotifications } from "./hooks/useActivityNotifications";
import { useCardNotifications } from "./sections/events/useCardNotifications";
import { useOpDoneNotifications } from "./hooks/useOpDoneNotifications";
import { useUsageThresholdNotifications } from "./hooks/useUsageThresholdNotifications";
import {
  consumeRecentTarget,
  dispatchOsNotification,
  type NotificationTarget,
} from "./lib/notify";
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
    clearDismissed,
    knownDismissedKeys,
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
          : op.kind === "session_move"
            ? "Moving session"
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

  // ⌃⌥⌘L — toggle developer mode globally. The combo requires all
  // four modifiers (Cmd + Alt + Ctrl + L), which is effectively
  // unreachable by accident; matches macOS's own deep-system-toggle
  // convention (⌃⌥⌘8 inverts screen colors, etc.). The settings
  // surface no longer renders a visible toggle, so this is the only
  // entry point. A toast confirms the new state since the toggle
  // is otherwise invisible.
  useEffect(() => {
    const onDevKey = (e: KeyboardEvent) => {
      if (!e.metaKey || !e.ctrlKey || !e.altKey) return;
      if (e.key !== "l" && e.key !== "L") return;
      e.preventDefault();
      const next = !readDevMode();
      writeDevMode(next);
      pushToast("info", next ? "Developer mode on" : "Developer mode off");
    };
    window.addEventListener("keydown", onDevKey);
    return () => window.removeEventListener("keydown", onDevKey);
  }, [pushToast]);

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
      const detail = (
        e as CustomEvent<{
          id?: string;
          sessionPath?: string;
          projectPath?: string;
        }>
      ).detail;
      const id = detail?.id;
      if (id && sectionIds.includes(id)) {
        if (detail.sessionPath) {
          setPendingSessionPath(detail.sessionPath);
        }
        if (detail.projectPath) {
          setPendingProjectPath(detail.projectPath);
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
          .catch((e) => toastError(pushToast, "Sync failed", e));
        return;
      }
      if (cmd === "app-menu:account:verify-all") {
        api
          .verifyAllAccounts()
          .then(() => {
            pushToast("info", "Verify all complete.");
            void refreshAccounts();
          })
          .catch((e) => toastError(pushToast, "Verify failed", e));
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
  // user selects a session hit, stash the target path and jump to
  // Projects. `ProjectsSection` consumes the pending path on mount,
  // resolves the owning project from the file's slug, and opens the
  // transcript in its master-detail pane. Previous implementation
  // raced via `setTimeout(0)` and could drop the selection on slow
  // mounts.
  useEffect(() => {
    function onGoto(ev: Event) {
      const detail = (ev as CustomEvent<{ filePath: string }>).detail;
      if (!detail?.filePath) return;
      // After the events-into-projects collapse, transcripts open
      // inside ProjectsSection's master-detail pane. ProjectsSection
      // derives the matching project from the file's slug when no
      // explicit projectPath is supplied — see its pending-consumer
      // effect.
      setPendingSessionPath(detail.filePath);
      setSection("projects");
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
  // Op-completion OS notifications. Fires when a long-running op
  // (verify_all, project rename, session prune/slim/share/move,
  // account login/register, clean projects, automation run)
  // terminates while the window is unfocused, gated by the
  // `notify_on_op_done` preference.
  useOpDoneNotifications();
  // Usage-threshold OS notifications. Listens for the
  // `usage-threshold-crossed` event emitted by the Rust-side
  // `usage_watcher` task; the watcher itself enforces the once-per-
  // (window × threshold) per-cycle policy. See
  // src/hooks/useUsageThresholdNotifications.ts for click routing.
  useUsageThresholdNotifications();

  // Notification-click router. The Tauri 2 desktop notification plugin
  // doesn't surface body-click events to JS (verified by reading
  // tauri-plugin-notification 2.3.3's desktop.rs — it spawns
  // notify_rust::Notification::show() and discards the handle). We
  // reconstruct intent via a focus-event heuristic: every dispatched
  // notification pushes its declared target into a small in-memory
  // queue with a 10-second TTL; whenever the window gains focus we
  // pop the most-recent unexpired entry.
  //
  // False-positive bound: a user who ignores a banner and opens
  // Claudepot manually within 10 s of dispatch gets routed to the
  // banner's target. Acceptable — the worst case is "navigated to a
  // section the user wasn't aiming for" and a single back-button (or
  // sidebar click) recovers. False-positives older than 10 s are
  // impossible by construction.
  useEffect(() => {
    const handler = () => {
      const target = consumeRecentTarget();
      if (!target) return;
      void routeNotificationTarget(target);
    };

    /** Translate a target into the matching internal navigation. The
     *  shell already owns `setSection` + `pendingSessionPath` state,
     *  so we ride the same `claudepot:navigate-section` event the
     *  rest of the app uses for cross-surface deep links. The `host`
     *  intent invokes the Rust command first and falls back to
     *  `app(projects/<sid>)` when the host can't be resolved. */
    const routeNotificationTarget = async (target: NotificationTarget) => {
      if (target.kind === "info") return;
      if (target.kind === "host") {
        try {
          const activated =
            await api.notificationActivateHostForSession(target.session_id);
          if (activated) return;
        } catch {
          // Backend command absent or failed — fall through to the
          // in-app deep link path. Never surface a toast: this code
          // runs on every focus event with a queued target, and a
          // permission-denied or stale-session path would be noise.
        }
        // Host unresolved → open the transcript inside Claudepot.
        window.dispatchEvent(
          new CustomEvent("claudepot:navigate-section", {
            detail: {
              id: "projects",
              sessionPath: undefined,
              projectPath: target.cwd,
            },
          }),
        );
        // Try to seed the session as well — projects is the owner.
        // The existing cp-activity-open-session pipe takes a session
        // id and resolves the transcript path through the live
        // snapshot, matching the tray's behavior.
        try {
          const snap = await api.sessionLiveSnapshot();
          const row = snap.find((s) => s.session_id === target.session_id);
          if (row?.transcript_path) {
            setPendingSessionPath(row.transcript_path);
          }
          if (row?.cwd) {
            setPendingProjectPath(row.cwd);
          } else {
            setPendingProjectPath(target.cwd);
          }
        } catch {
          /* no-tauri or snapshot failed — projectPath alone is fine */
        }
        return;
      }
      // target.kind === "app"
      const r = target.route;
      if (r.section === "accounts") {
        setSection("accounts");
        if (r.email) {
          // The Accounts focus listener (AccountsSection.tsx ~L172)
          // expects `event.detail` to be a bare uuid string, not an
          // object — it scrolls to `[data-account-uuid="${detail}"]`.
          // Resolve email → uuid here using the same `accounts`
          // snapshot the rest of the shell renders against. If the
          // email isn't in the live list (just removed, or the
          // notification fired against a stale snapshot), the section
          // switch still happens; only the scroll-into-view is lost.
          const acct = accounts.find((a) => a.email === r.email);
          if (acct?.uuid) {
            window.dispatchEvent(
              new CustomEvent("cp-focus-account", { detail: acct.uuid }),
            );
          }
        }
        return;
      }
      if (r.section === "projects") {
        if (r.session_id) {
          try {
            const snap = await api.sessionLiveSnapshot();
            const row = snap.find((s) => s.session_id === r.session_id);
            if (row?.transcript_path) setPendingSessionPath(row.transcript_path);
            if (row?.cwd) setPendingProjectPath(row.cwd);
            else if (r.cwd) setPendingProjectPath(r.cwd);
          } catch {
            if (r.cwd) setPendingProjectPath(r.cwd);
          }
        } else if (r.cwd) {
          setPendingProjectPath(r.cwd);
        }
        setSection("projects");
        return;
      }
      if (r.section === "settings" || r.section === "events") {
        setSection(r.section);
      }
    };

    // Bell-icon popover: a click on a logged entry dispatches the
    // entry's stored target through this same routing function.
    // The popover doesn't import routeNotificationTarget directly —
    // it lives inside this useEffect closure — so we round-trip
    // through a window event. Same shape as the focus path; if the
    // target lacks a click destination the popover never dispatches.
    const popoverHandler = (ev: Event) => {
      const detail = (ev as CustomEvent<{ target?: NotificationTarget }>)
        .detail;
      if (!detail?.target) return;
      void routeNotificationTarget(detail.target);
    };

    window.addEventListener("focus", handler);
    window.addEventListener(
      "claudepot:notification-log-target",
      popoverHandler,
    );
    return () => {
      window.removeEventListener("focus", handler);
      window.removeEventListener(
        "claudepot:notification-log-target",
        popoverHandler,
      );
    };
    // `accounts` is read inside routeNotificationTarget (the
    // accounts-route branch resolves email → uuid against this list).
    // Without it in the dep array the closure captures whatever
    // accounts looked like at first mount, and account adds/removes
    // would silently miss the focus dispatch. Re-running on accounts
    // change is cheap — just rebinds two window listeners — and the
    // route function only fires when a notification clicks through.
  }, [setSection, accounts]);

  // Mirror the alert count into the tray so tray-only users see a
  // persistent signal when the window is hidden. Diffed against a ref
  // because the count is recomputed on every live-snapshot tick — we
  // only fire the IPC when the integer actually changes. Errors are
  // swallowed: the tray simply stays at its last-known value.
  const lastTrayCountRef = useRef<number | null>(null);
  useEffect(() => {
    if (lastTrayCountRef.current === activityAlerts) return;
    lastTrayCountRef.current = activityAlerts;
    api
      .traySetAlertCount(activityAlerts)
      .catch(() => {
        /* tray unmanaged in test harness — keep going */
      });
  }, [activityAlerts]);

  // Tray → Activity row click lands on the Tauri event
  // `cp-activity-open-session` with the session id as payload.
  // Resolve to a transcript path via the live runtime's snapshot so
  // the existing Sessions deep-link pipe handles routing. If the
  // session isn't in the snapshot (already ended between click and
  // handler), just switch to Sessions.
  useEffect(() => {
    // active-flag pattern: if cleanup runs before listen() resolves
    // (StrictMode double-mount, fast unmount), the .then handler
    // detaches the listener itself instead of leaking it.
    let active = true;
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
        if (row?.cwd) {
          setPendingProjectPath(row.cwd);
        }
      } catch {
        /* fallback to just switching */
      }
      // Sessions live inside Projects after the events-into-projects
      // collapse; the live snapshot already carries `cwd` so the
      // pending-consumer can pick the right project on first paint.
      setSection("projects");
    })
      .then((fn) => {
        if (!active) fn();
        else unlisten = fn;
      })
      .catch(() => {
        /* no-tauri env */
      });
    return () => {
      active = false;
      unlisten?.();
    };
  }, [setSection]);

  // Tray Desktop actions route through the shell's confirmation
  // modal: the tray itself can't render a modal, so it emits events
  // the main window converts into the same DesktopConfirmDialog
  // flow as the in-window context menu + palette.
  useEffect(() => {
    // active-flag pattern: same fix as the cp-activity-open-session
    // effect above — without it, a cleanup landing before either
    // listen() promise resolves would leak the listener.
    let active = true;
    let unlistenClear: (() => void) | null = null;
    let unlistenBind: (() => void) | null = null;
    listen("cp-tray-desktop-clear", () => requestDesktopSignOut())
      .then((fn) => {
        if (!active) fn();
        else unlistenClear = fn;
      })
      .catch(() => {});
    listen("cp-tray-desktop-bind", () => {
      // Route to Accounts so the adoption banner / context menu is
      // visible — the user picks a target account there.
      setSection("accounts");
    })
      .then((fn) => {
        if (!active) fn();
        else unlistenBind = fn;
      })
      .catch(() => {});
    return () => {
      active = false;
      unlistenClear?.();
      unlistenBind?.();
    };
  }, [requestDesktopSignOut, setSection]);

  // Tray → CLI switch feedback. The tray now performs the swap with
  // `force=true` and emits `tray-cli-switched` with `{ to_email,
  // from_email, cc_was_running }`. Two channels surface the result so
  // the user is never left wondering whether the click landed:
  //
  //   - Toast in-window with a 10 s Undo button. Visible immediately
  //     when the user is on Claudepot, and still visible (paused
  //     animation aside) when they bring the window forward.
  //   - OS notification when the window is in the background. The
  //     notification dispatcher gates on `document.hasFocus()` so
  //     foregrounded users never get duplicate signals. Clicking the
  //     banner deep-links to Accounts where the toast (still alive)
  //     carries the actual Undo affordance — Tauri's desktop
  //     notification plugin doesn't expose action buttons, so the
  //     in-window toast is the only place an Undo click can live.
  //
  // The cc-was-running caveat is appended to both surfaces: a forced
  // swap can be silently reverted by CC's next token refresh, and the
  // user has to know to quit + restart Claude Code.
  type TrayCliSwitchedPayload = {
    to_email: string;
    from_email: string | null;
    cc_was_running: boolean;
  };
  // Refs let the listener stay registered for the shell's lifetime
  // even though the Undo closure needs the latest accounts and
  // actions. Without this, the effect re-subscribed every time
  // `accounts` changed (which the handler itself triggers via
  // `refreshAccounts`), opening a small window between cleanup and
  // the next async `listen()` resolution where a tray event could
  // land unobserved.
  const accountsRef = useRef(accounts);
  const actionsRef = useRef(actions);
  const pushToastRef = useRef(pushToast);
  const refreshAccountsRef = useRef(refreshAccounts);
  useEffect(() => {
    accountsRef.current = accounts;
    actionsRef.current = actions;
    pushToastRef.current = pushToast;
    refreshAccountsRef.current = refreshAccounts;
  }, [accounts, actions, pushToast, refreshAccounts]);
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;
    listen<TrayCliSwitchedPayload>("tray-cli-switched", (ev) => {
      const p = ev.payload;
      // Defensive: tolerate older payloads (none / shape drift) by
      // refreshing and bailing — the user still sees the active-flag
      // change land in the cards, just without the toast/notification.
      if (!p || typeof p.to_email !== "string") {
        void refreshAccountsRef.current();
        return;
      }
      void refreshAccountsRef.current();

      const caveat = p.cc_was_running
        ? " — restart Claude Code to apply"
        : "";
      const undoFn = p.from_email
        ? () => {
            const prev = accountsRef.current.find(
              (a) => a.email === p.from_email,
            );
            if (!prev) {
              pushToastRef.current(
                "error",
                `Undo failed: ${p.from_email} not found`,
              );
              return;
            }
            // Mirror the tray's force semantics on undo: the user is
            // already inside the same one-click flow, the SplitBrain
            // modal would just re-introduce the visibility problem
            // this whole change exists to fix.
            void actionsRef.current.useCli(prev, true);
          }
        : undefined;
      pushToastRef.current(
        "info",
        `CLI → ${p.to_email}${caveat}`,
        undoFn,
        { undoLabel: "Undo", undoMs: 10_000 },
      );

      void dispatchOsNotification(
        `CLI switched to ${p.to_email}`,
        p.cc_was_running
          ? "Restart Claude Code to apply. Open Claudepot to undo."
          : "Open Claudepot within 10 s to undo.",
        {
          target: { kind: "app", route: { section: "accounts" } },
          group: "claudepot.cli-switch",
          sound: "default",
        },
      );
    })
      .then((fn) => {
        if (!active) fn();
        else unlisten = fn;
      })
      .catch(() => {});
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  // Tray → CLI switch failure. Same hidden-window concern as the
  // success path: the in-window error toast is invisible when the
  // user is in another app, so mirror to OS notification. Failures
  // are rare (live conflicts are now forced past, so the residual is
  // store/keychain class), and don't carry an Undo affordance.
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;
    listen<string>("tray-cli-switch-failed", (ev) => {
      const detail =
        typeof ev?.payload === "string" && ev.payload.length > 0
          ? ev.payload
          : "unknown";
      pushToast("error", `Switch failed: ${detail}`);
      void dispatchOsNotification("CLI switch failed", detail, {
        target: { kind: "app", route: { section: "accounts" } },
        group: "claudepot.cli-switch",
      });
    })
      .then((fn) => {
        if (!active) fn();
        else unlisten = fn;
      })
      .catch(() => {});
    return () => {
      active = false;
      unlisten?.();
    };
  }, [pushToast]);

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
      toastError(pushToast, "Unlock failed", e);
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
        toastError(pushToast, "Import failed", e);
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

  // Snooze auto-clear: when an issue id is no longer present in
  // `rawIssues`, the underlying condition has resolved. Drop its
  // entry from the dismissed-issues store so a re-occurrence later
  // shows the banner immediately instead of being silently re-snoozed
  // against the stale 24 h timer.
  //
  // The first effect run reconciles localStorage's dismissed-store
  // against the live rawIssues — this catches stale entries left
  // over from a previous renderer lifetime (user dismissed issue X,
  // closed app, condition resolved while closed, app reopened
  // before the 24 h TTL would expire X). Subsequent runs diff
  // against a ref of the previous-tick id set so we only call
  // `clearDismissed` for ids that actually disappeared this tick.
  const seenIssueIdsRef = useRef<Set<string> | null>(null);
  useEffect(() => {
    const current = new Set(rawIssues.map((i) => i.id));
    const prev = seenIssueIdsRef.current;
    if (prev === null) {
      // First run — reconcile against persisted snooze entries from
      // a prior renderer lifetime, not just the in-memory ref.
      for (const id of knownDismissedKeys()) {
        if (!current.has(id)) clearDismissed(id);
      }
    } else {
      for (const id of prev) {
        if (!current.has(id)) clearDismissed(id);
      }
    }
    seenIssueIdsRef.current = current;
  }, [rawIssues, clearDismissed, knownDismissedKeys]);

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
        if (s.cwd) setPendingProjectPath(s.cwd);
        // Sessions land inside Projects after the events-into-
        // projects collapse; ProjectsSection's pending consumer
        // selects the matching project and opens the transcript.
        setSection("projects");
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
        height: "var(--viewport-h-full)",
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
            // badge count on the Activity nav row — the merged
            // dashboard + events surface that now owns "what's
            // happening right now."
            events: activityAlerts || undefined,
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
                  tab switch. The label is the user-facing section name
                  from the registry. */}
              {section === "accounts" && (
                <ErrorBoundary key="accounts" label="Accounts">
                  <AccountsSection onNavigate={setSection} />
                </ErrorBoundary>
              )}
              {section === "projects" && (
                <ErrorBoundary key="projects" label="Projects">
                  <ProjectsSection
                    subRoute={subRoute}
                    onSubRouteChange={setSubRoute}
                    pendingProjectPath={pendingProjectPath}
                    pendingSessionPath={pendingSessionPath}
                    onPendingConsumed={() => {
                      setPendingProjectPath(null);
                      setPendingSessionPath(null);
                    }}
                  />
                </ErrorBoundary>
              )}
              {section === "events" && (
                <ErrorBoundary key="events" label="Activities">
                  <EventsSection />
                </ErrorBoundary>
              )}
              {section === "global" && (
                <ErrorBoundary key="global" label="Global">
                  <GlobalSection
                    subRoute={subRoute}
                    onSubRouteChange={setSubRoute}
                  />
                </ErrorBoundary>
              )}
              {section === "keys" && (
                <ErrorBoundary key="keys" label="Keys">
                  <KeysSection />
                </ErrorBoundary>
              )}
              {section === "third-party" && (
                <ErrorBoundary key="third-party" label="Third-parties">
                  <ThirdPartySection />
                </ErrorBoundary>
              )}
              {section === "automations" && (
                <ErrorBoundary key="automations" label="Automations">
                  <AutomationsSection />
                </ErrorBoundary>
              )}
              {section === "settings" && (
                <ErrorBoundary key="settings" label="Settings">
                  <SettingsSection />
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
      />

      {activeOp && (
        <Suspense fallback={null}>
          <OperationProgressModal
            key={activeOp.opId}
            opId={activeOp.opId}
            title={activeOp.title}
            phases={activeOp.phases ?? PROJECT_MOVE_PHASES}
            fetchStatus={activeOp.fetchStatus ?? api.projectMoveStatus}
            renderResult={activeOp.renderResult ?? renderProjectMoveResult}
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
              toastError(pushToast, "Desktop launch failed", e);
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
