import { useEffect, useRef } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api } from "../api";
import { dispatchOsNotification } from "../lib/notify";
import type { Preferences } from "../types";

/**
 * `useOpDoneNotifications` — fires an OS notification when a long-
 * running op terminates. Listens on the global `cp-op-terminal`
 * channel emitted by `src-tauri/src/ops.rs::emit_terminal`, which is
 * the single point through which every op (verify_all, project rename,
 * session prune/slim/share/move, account login/register, clean
 * projects, automation run) signals completion.
 *
 * Gated on:
 *   * `notify_on_op_done` preference (default off, opt-in)
 *   * Window-blurred — the shared dispatcher applies the focus gate;
 *     when the window has focus the in-app `RunningOpsChip` already
 *     animates the terminal state, so an additional OS toast would
 *     double-signal.
 *   * OS notification permission — handled by the shared dispatcher.
 *
 * The `cp-op-terminal` event payload mirrors the typed
 * `OpTerminalEvent` struct on the backend: `{op_id, kind, status,
 * label, error?}`. The label is built backend-side so the
 * notification body reads like the running-ops popover without a
 * second IPC round-trip.
 */
interface OpTerminalWire {
  op_id: string;
  kind: string;
  status: "complete" | "error";
  label: string;
  error?: string;
}

function dispatchPayload(payload: OpTerminalWire): void {
  const title =
    payload.status === "error"
      ? `Operation failed: ${payload.label}`
      : payload.label;
  // Trim error detail to a one-liner — the backend already
  // redacts sk-ant-* upstream, but the body should still stay
  // readable in a tray-corner banner. Truncate at 200 chars.
  const errBody =
    payload.error && payload.error.length > 200
      ? `${payload.error.slice(0, 199)}…`
      : payload.error;
  const body =
    payload.status === "error" && errBody
      ? errBody
      : payload.status === "error"
        ? "See the running-ops chip for details."
        : "Done.";
  // dedupeKey per op_id (each op should only ever fire one
  // terminal notification, but the bucket is the right defense
  // against a future bug that double-emits). group by op kind
  // so e.g. five back-to-back verify_all completions stack.
  // Click intent: open Claudepot — op completions ARE about
  // Claudepot's own state (verify-all, project rename, prune,
  // share, etc.). No deeper deep-link target today; the running-
  // ops chip in the shell already animates the terminal state, so
  // landing on the last-active section is the right behavior.
  void dispatchOsNotification(title, body, {
    dedupeKey: `op:${payload.op_id}`,
    group: `op:${payload.kind}`,
    sound: "default",
    target: { kind: "info" },
  });
}

export function useOpDoneNotifications(): void {
  const enabledRef = useRef(false);
  // Buffer for terminal events that arrive before `preferencesGet()`
  // resolves. Without this, the very first op a user triggers right
  // after launch is dropped because `enabledRef.current` is still
  // false during the initial IPC round-trip. Once prefs load, we
  // flush the buffer through the dispatcher iff `notify_on_op_done`
  // is enabled. Dropped silently when prefs load with the toggle off.
  const prefsLoadedRef = useRef(false);
  const pendingRef = useRef<OpTerminalWire[]>([]);

  useEffect(() => {
    let active = true;
    let unlistenPrefs: UnlistenFn | null = null;

    const apply = (p: Preferences) => {
      if (!active) return;
      const enabled = !!p.notify_on_op_done;
      enabledRef.current = enabled;
      const wasLoaded = prefsLoadedRef.current;
      prefsLoadedRef.current = true;
      // Flush buffered events the first time prefs land. Re-applying
      // (cp-prefs-changed event after the initial load) doesn't
      // re-flush — buffered events represent "fired before initial
      // load", not "fired before any pref change".
      if (!wasLoaded && pendingRef.current.length > 0) {
        const drained = pendingRef.current;
        pendingRef.current = [];
        if (enabled) {
          for (const ev of drained) dispatchPayload(ev);
        }
      }
    };

    api
      .preferencesGet()
      .then(apply)
      .catch(() => {
        // Non-Tauri env: mark prefs as "loaded" so the listener stops
        // buffering. enabledRef stays false, so nothing dispatches —
        // the pending buffer drains to /dev/null cleanly on next
        // event arrival.
        if (active) {
          prefsLoadedRef.current = true;
          pendingRef.current = [];
        }
      });

    listen<Preferences>("cp-prefs-changed", (ev) => {
      if (ev.payload) apply(ev.payload);
    })
      .then((fn) => {
        if (!active) fn();
        else unlistenPrefs = fn;
      })
      .catch(() => {
        /* no-tauri env */
      });

    return () => {
      active = false;
      unlistenPrefs?.();
    };
  }, []);

  useEffect(() => {
    let active = true;
    let unlisten: UnlistenFn | null = null;

    listen<OpTerminalWire>("cp-op-terminal", (ev) => {
      const payload = ev.payload;
      if (!payload) return;
      if (!prefsLoadedRef.current) {
        // Prefs haven't loaded yet — buffer for replay. Cap the
        // buffer at 32 to avoid an unbounded grow if prefsGet hangs
        // forever; in practice prefs land within a few hundred ms.
        if (pendingRef.current.length < 32) {
          pendingRef.current.push(payload);
        }
        return;
      }
      if (!enabledRef.current) return;
      dispatchPayload(payload);
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
  }, []);
}
