import { useEffect, useRef } from "react";
import type { Event as TauriEvent } from "@tauri-apps/api/event";
import { api } from "../api";
import { useTauriEvent, useTauriEvents } from "./useTauriEvent";
import type { EmitFn } from "../lib/notifications/dispatch";
import type { AccountSummary } from "../types";

/**
 * Tray → main-window bridge, extracted from AppShell. Owns every
 * tray-originated concern:
 *
 *   - mirrors the activity alert count into the tray badge
 *   - `cp-activity-open-session` — tray Activity row click
 *   - `cp-tray-desktop-clear` / `cp-tray-desktop-bind` — tray Desktop
 *     actions routed through the shell's confirmation modal flow
 *   - `tray-cli-switched` / `tray-cli-switch-failed` — one-click CLI
 *     swap feedback (toast with Undo + OS banner mirror)
 *
 * Every subscription is registered once for the shell's lifetime —
 * useTauriEvent(s) hold handlers in refs, so the unstable arg
 * identities (accounts, actions, …) never re-wire a channel. This
 * replaces the old hand-rolled five-ref mirror block AND fixes the
 * `[pushToast]`-dep re-subscription on the failure channel.
 */

/** Payload of `tray-cli-switched` (see src-tauri tray swap path). */
interface TrayCliSwitchedPayload {
  to_email: string;
  from_email: string | null;
  cc_was_running: boolean;
}

export function useTrayBridge(args: {
  /** Alerting-session count (errored / stuck / waiting). */
  alertCount: number;
  setSection: (id: string) => void;
  setPendingSessionPath: (path: string | null) => void;
  setPendingProjectPath: (path: string | null) => void;
  requestDesktopSignOut: () => void;
  accounts: AccountSummary[];
  actions: { useCli: (a: AccountSummary, force?: boolean) => Promise<void> };
  pushToast: (kind: "info" | "error", text: string) => void;
  emit: EmitFn;
  refreshAccounts: () => Promise<void>;
}): void {
  const {
    alertCount,
    setSection,
    setPendingSessionPath,
    setPendingProjectPath,
    requestDesktopSignOut,
    accounts,
    actions,
    pushToast,
    emit,
    refreshAccounts,
  } = args;

  // Mirror the alert count into the tray so tray-only users see a
  // persistent signal when the window is hidden. Diffed against a ref
  // because the count is recomputed on every live-snapshot tick — we
  // only fire the IPC when the integer actually changes. Errors are
  // swallowed: the tray simply stays at its last-known value.
  const lastTrayCountRef = useRef<number | null>(null);
  useEffect(() => {
    if (lastTrayCountRef.current === alertCount) return;
    lastTrayCountRef.current = alertCount;
    api.traySetAlertCount(alertCount).catch(() => {
      /* tray unmanaged in test harness — keep going */
    });
  }, [alertCount]);

  // The Undo closure on a tray CLI swap runs up to 10 s after the
  // event arrived; by then `refreshAccounts` (triggered by the swap
  // itself) has usually replaced the accounts list. Read through
  // refs at press time so Undo acts on the freshest snapshot, not
  // the one captured when the toast was created.
  const accountsRef = useRef(accounts);
  accountsRef.current = accounts;
  const actionsRef = useRef(actions);
  actionsRef.current = actions;
  const pushToastRef = useRef(pushToast);
  pushToastRef.current = pushToast;

  // Tray → Activity row click lands on the Tauri event
  // `cp-activity-open-session` with the session id as payload.
  // Resolve to a transcript path via the live runtime's snapshot so
  // the existing Sessions deep-link pipe handles routing. If the
  // session isn't in the snapshot (already ended between click and
  // handler), just switch to Sessions.
  useTauriEvent<string>("cp-activity-open-session", (ev) => {
    void (async () => {
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
    })();
  });

  // Tray Desktop actions route through the shell's confirmation
  // modal: the tray itself can't render a modal, so it emits events
  // the main window converts into the same DesktopConfirmDialog
  // flow as the in-window context menu + palette.
  //
  // Tray → CLI switch feedback. The tray performs the swap with
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
  //
  // Failures are rare (live conflicts are forced past, so the
  // residual is store/keychain class) and don't carry an Undo
  // affordance; the error toast is mirrored to an OS notification
  // for the same hidden-window reason.
  useTauriEvents({
    "cp-tray-desktop-clear": () => requestDesktopSignOut(),
    "cp-tray-desktop-bind": () => {
      // Route to Accounts so the adoption banner / context menu is
      // visible — the user picks a target account there.
      setSection("accounts");
    },
    "tray-cli-switched": (ev: TauriEvent<TrayCliSwitchedPayload>) => {
      const p = ev.payload;
      // Defensive: tolerate older payloads (none / shape drift) by
      // refreshing and bailing — the user still sees the active-flag
      // change land in the cards, just without the toast/notification.
      if (!p || typeof p.to_email !== "string") {
        void refreshAccounts();
        return;
      }
      void refreshAccounts();

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
      // Tray-driven CLI switch: route through emit() so the bell
      // records a routed entry. accountSwitched (P2) toasts
      // in-app; we add osOverride=true via the kind contract by
      // setting category=accountSwitched and letting routing apply.
      void emit({
        category: "accountSwitched",
        title: `CLI → ${p.to_email}${caveat}`,
        body: p.cc_was_running
          ? "Restart Claude Code to apply. Open Claudepot to undo."
          : "Open Claudepot within 10 s to undo.",
        target: { kind: "app", route: { section: "accounts" } },
        toastAction: undoFn
          ? { label: "Undo", onPress: undoFn, timeoutMs: 10_000 }
          : undefined,
      });
    },
    "tray-cli-switch-failed": (ev: TauriEvent<string>) => {
      const detail =
        typeof ev?.payload === "string" && ev.payload.length > 0
          ? ev.payload
          : "unknown";
      void emit({
        category: "accountSwitched",
        kind: "error",
        title: "CLI switch failed",
        body: detail,
        target: { kind: "app", route: { section: "accounts" } },
      });
    },
  });
}
