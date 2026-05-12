import { useEffect, useRef } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEmit } from "../providers/AppStateProvider";
import type { EmitFn } from "../lib/notifications/dispatch";

/**
 * `useOpDoneNotifications` — fires through `emit()` when a long-
 * running op terminates. Listens on the global `cp-op-terminal`
 * channel emitted by `src-tauri/src/ops.rs::emit_terminal`, which is
 * the single point through which every op (verify_all, project rename,
 * session prune/slim/share/move, account login/register, clean
 * projects, automation run) signals completion.
 *
 * Phase 3 migration: the legacy hook called `dispatchOsNotification`
 * directly and gated on the `notify_on_op_done` scalar pref. The
 * routing now flows through `emit()` with `category=opDoneUnfocused`
 * — the `CategoryPrefs.enabled` flag (dual-write-synced with the old
 * scalar via Phase 1.5) gates dispatch. Audit issue #5 (buffer
 * asymmetry where a pref change AFTER the initial load wouldn't flush
 * pending events) is fixed by removing the buffer entirely: emit()
 * reads the pref cache synchronously each call, so there's nothing
 * to buffer — late events just dispatch correctly.
 *
 * Gated on:
 *   * `CategoryPrefs(opDoneUnfocused).enabled` (default false,
 *     migrated from `notify_on_op_done` scalar)
 *   * Window-blurred — the routing's P1 default for
 *     opDoneUnfocused doesn't set `ignoreFocus`, and the OS
 *     dispatcher's focus gate suppresses the banner when the window
 *     is focused. The in-app `RunningOpsChip` already animates the
 *     terminal state, so a focused-window OS toast would
 *     double-signal.
 *   * OS notification permission — handled by the shared dispatcher.
 */
interface OpTerminalWire {
  op_id: string;
  kind: string;
  status: "complete" | "error";
  label: string;
  error?: string;
}

function dispatchPayload(emit: EmitFn, payload: OpTerminalWire): void {
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
  // against a future bug that double-emits).
  // Click intent: open Claudepot — op completions ARE about
  // Claudepot's own state. No deeper deep-link target today; the
  // running-ops chip in the shell already animates the terminal
  // state, so landing on the last-active section is the right
  // behavior.
  void emit({
    category: "opDoneUnfocused",
    kind: payload.status === "error" ? "error" : "notice",
    title,
    body,
    dedupeKey: `op:${payload.op_id}`,
    target: { kind: "info" },
  });
}

export function useOpDoneNotifications(): void {
  const emit = useEmit();
  // emit() reads CategoryPrefs synchronously from the renderer
  // cache (hydrated on AppStateProvider mount). No more buffering
  // for pref-load — emit() handles unloaded-cache by treating
  // categories as enabled, which matches the audit-validated
  // intent ("don't drop the user's first event after launch").
  const emitRef = useRef(emit);
  emitRef.current = emit;

  useEffect(() => {
    let active = true;
    let unlisten: UnlistenFn | null = null;

    listen<OpTerminalWire>("cp-op-terminal", (ev) => {
      const payload = ev.payload;
      if (!payload) return;
      dispatchPayload(emitRef.current, payload);
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
