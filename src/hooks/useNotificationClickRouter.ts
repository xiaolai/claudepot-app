import { useEffect } from "react";
import { api } from "../api";
import {
  consumeRecentTarget,
  type NotificationTarget,
} from "../lib/notify";
import type { AccountSummary } from "../types";

/**
 * Notification-click router, extracted from AppShell.
 *
 * The Tauri 2 desktop notification plugin doesn't surface body-click
 * events to JS (verified by reading tauri-plugin-notification 2.3.3's
 * desktop.rs — it spawns notify_rust::Notification::show() and
 * discards the handle). We reconstruct intent via a focus-event
 * heuristic: every dispatched notification pushes its declared target
 * into a small in-memory queue with a 10-second TTL; whenever the
 * window gains focus we pop the most-recent unexpired entry.
 *
 * False-positive bound: a user who ignores a banner and opens
 * Claudepot manually within 10 s of dispatch gets routed to the
 * banner's target. Acceptable — the worst case is "navigated to a
 * section the user wasn't aiming for" and a single back-button (or
 * sidebar click) recovers. False-positives older than 10 s are
 * impossible by construction.
 */
export function useNotificationClickRouter(args: {
  setSection: (id: string) => void;
  setPendingSessionPath: (path: string | null) => void;
  setPendingProjectPath: (path: string | null) => void;
  accounts: AccountSummary[];
}): void {
  const {
    setSection,
    setPendingSessionPath,
    setPendingProjectPath,
    accounts,
  } = args;

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
  }, [
    setSection,
    setPendingSessionPath,
    setPendingProjectPath,
    accounts,
  ]);
}
