import { useEmit } from "../providers/AppStateProvider";
import { useTauriEvent } from "./useTauriEvent";
import type { EmitFn } from "../lib/notifications/dispatch";
import { redactSecrets } from "../lib/redactSecrets";

/**
 * `useOpDoneNotifications` — fires through `emit()` when a long-
 * running op terminates. Listens on the global `cp-op-terminal`
 * channel emitted by `src-tauri/src/ops.rs::emit_terminal`, which is
 * the single point through which every op (verify_all, project rename,
 * session prune/slim/share/move, account login/register, clean
 * projects, agent run) signals completion.
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
  // Defense-in-depth secret redaction. The backend redacts sk-ant-*
  // upstream in most paths, but a panic backtrace or third-party
  // crate error string could still surface a raw token. Redact
  // client-side before persisting to the bell log or rendering an
  // OS banner — `.claude/rules/design.md` non-negotiable: credentials
  // never rendered.
  const redactedError = payload.error
    ? redactSecrets(payload.error)
    : payload.error;
  // Trim error detail to a one-liner — keep the toast readable in
  // a tray-corner banner. Truncate at 200 chars.
  const errBody =
    redactedError && redactedError.length > 200
      ? `${redactedError.slice(0, 199)}…`
      : redactedError;
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
  // useTauriEvent holds the handler in a ref, so the per-render
  // closure always sees the latest emit without re-subscribing.
  useTauriEvent<OpTerminalWire>("cp-op-terminal", (ev) => {
    const payload = ev.payload;
    if (!payload) return;
    dispatchPayload(emit, payload);
  });
}
