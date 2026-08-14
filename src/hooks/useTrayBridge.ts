import { useEffect, useRef } from "react";
import type { Event as TauriEvent } from "@tauri-apps/api/event";
import { api } from "../api";
import { i18n } from "../lib/i18n";
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
 *   - `tray-desktop-switched` / `tray-desktop-switch-failed` /
 *     `tray-desktop-launch-failed` / `desktop-reconciled` — the same
 *     feedback for the Desktop slot
 *
 * Every channel the backend emits for a tray action is handled here.
 * That is now enforced by `cargo xtask verify-docs`, which fails when
 * a channel declared in `src-tauri/src/events.rs` has no subscriber
 * anywhere in `src/` — the four Desktop channels above were emitted
 * for releases with nobody listening.
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

/** Payload of `tray-desktop-switched` (`events.rs::TrayDesktopSwitched`). */
interface TrayDesktopSwitchedPayload {
  to_email: string;
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

      // Two whole-phrase keys rather than a translated suffix appended
      // to a translated stem — the caveat doesn't sit at the end of the
      // sentence in every language.
      const title = p.cc_was_running
        ? i18n.t("tray.cliSwitchedTitleRestart", { email: p.to_email })
        : i18n.t("tray.cliSwitchedTitle", { email: p.to_email });
      const undoFn = p.from_email
        ? () => {
            const prev = accountsRef.current.find(
              (a) => a.email === p.from_email,
            );
            if (!prev) {
              pushToastRef.current(
                "error",
                i18n.t("tray.undoFailed", { email: p.from_email }),
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
        title,
        body: p.cc_was_running
          ? i18n.t("tray.cliSwitchedBodyRestart")
          : i18n.t("tray.cliSwitchedBodyUndo"),
        target: { kind: "app", route: { section: "accounts" } },
        toastAction: undoFn
          ? { label: i18n.t("ui.undo"), onPress: undoFn, timeoutMs: 10_000 }
          : undefined,
      });
    },
    "tray-cli-switch-failed": (ev: TauriEvent<string>) => {
      void emit({
        category: "accountSwitched",
        kind: "error",
        title: i18n.t("account.cliSwitchFailed"),
        body: detailOf(ev),
        target: { kind: "app", route: { section: "accounts" } },
      });
    },

    // Tray → Desktop feedback. The CLI channels above had all of this
    // and the Desktop ones had none: nothing in the renderer
    // subscribed, so a tray Desktop swap left the account cards
    // showing the previous binding, and a FAILED swap produced no
    // signal anywhere — no toast, no banner, no log the user can see.
    // The only recovery was `useDesktopIdentitySync`, which is
    // throttled to one probe per five minutes AND only fires on window
    // focus, so a swap performed while the window was already visible
    // stayed invisible indefinitely.
    //
    // No Undo here, unlike the CLI swap. Undoing a Desktop switch is
    // not symmetric with performing one — the previous session's files
    // are in a snapshot dir that may or may not still hold them — so
    // offering a one-click reversal would promise more than the
    // backend guarantees.
    "tray-desktop-switched": (ev: TauriEvent<TrayDesktopSwitchedPayload>) => {
      const p = ev.payload;
      // Always refresh, even when the payload is unrecognizable: the
      // `is_desktop_active` flags in the cards are stale either way,
      // and a stale badge is the more misleading of the two failures.
      void refreshAccounts();
      if (!p || typeof p.to_email !== "string") return;
      void emit({
        category: "accountSwitched",
        title: i18n.t("tray.desktopSwitchedTitle", { email: p.to_email }),
        // The tray always swaps with `no_launch=true`, so Desktop is
        // never relaunched by this path — say so, or the user stares
        // at a Desktop window still showing the old account.
        body: i18n.t("tray.desktopSwitchedBody"),
        target: { kind: "app", route: { section: "accounts" } },
      });
    },
    "tray-desktop-switch-failed": (ev: TauriEvent<string>) => {
      void emit({
        category: "accountSwitched",
        kind: "error",
        title: i18n.t("account.desktopSwitchFailed"),
        body: detailOf(ev),
        target: { kind: "app", route: { section: "accounts" } },
      });
    },
    "tray-desktop-launch-failed": (ev: TauriEvent<string>) => {
      void emit({
        category: "accountSwitched",
        kind: "error",
        // `components:` prefix — `toasts.*` is not in the default
        // `common` namespace, and the typed `t()` is what caught it.
        title: i18n.t("components:toasts.desktopLaunchFailed"),
        body: detailOf(ev),
        target: { kind: "app", route: { section: "accounts" } },
      });
    },
    // Reconcile flips `is_desktop_active` in the store, so the cards
    // are stale whenever it changed anything. Silent at zero flips:
    // the tray runs this on demand and "nothing needed fixing" is not
    // worth a notification.
    "desktop-reconciled": (ev: TauriEvent<number>) => {
      const flips = typeof ev?.payload === "number" ? ev.payload : 0;
      if (flips <= 0) return;
      void refreshAccounts();
      void emit({
        category: "accountSwitched",
        title: i18n.t("tray.desktopReconciled", { count: flips }),
        target: { kind: "app", route: { section: "accounts" } },
      });
    },
  });
}

/**
 * Error text from a string-payload channel, falling back to "unknown".
 *
 * Four channels needed the identical guard; the fallback matters
 * because an empty payload would otherwise render an error toast with
 * a blank body, which reads as a UI bug rather than as a failed swap.
 */
function detailOf(ev: TauriEvent<string>): string {
  return typeof ev?.payload === "string" && ev.payload.length > 0
    ? ev.payload
    : i18n.t("ui.unknown");
}
