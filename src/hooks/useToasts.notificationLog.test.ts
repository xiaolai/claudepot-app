// Capture-instrumentation contract: every `pushToast` must fire one
// `notification_log_append` IPC so the bell-icon popover shows the
// toast after it auto-dismisses.
//
// We test this separately from `useToasts.test.ts` (which uses fake
// timers and never imports the Tauri shim) so the dispatch path stays
// honest without disturbing the existing autodismiss tests.

import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";

type InvokeArgs = [cmd: string, args?: unknown];
const invokeSpy = vi.fn<(...args: InvokeArgs) => Promise<unknown>>(
  async () => undefined,
);

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: InvokeArgs) => invokeSpy(...args),
}));

import { useToasts } from "./useToasts";

describe("useToasts — notification log capture", () => {
  beforeEach(() => {
    invokeSpy.mockClear();
    invokeSpy.mockImplementation(async () => undefined);
  });

  it("fires notification_log_append on every pushToast", () => {
    const { result } = renderHook(() => useToasts());
    act(() => result.current.pushToast("info", "Switched to bob@x.com"));

    // Filter to just the log-append IPCs in case other invoke
    // sites fire during this hook's lifecycle.
    const calls = invokeSpy.mock.calls.filter(
      (c) => c[0] === "notification_log_append",
    );
    expect(calls).toHaveLength(1);
    expect(calls[0][1]).toEqual({
      args: {
        source: "toast",
        kind: "info",
        title: "Switched to bob@x.com",
        body: "",
        target: null,
      },
    });
  });

  it("preserves error kind on the wire", () => {
    const { result } = renderHook(() => useToasts());
    act(() => result.current.pushToast("error", "Repair failed"));

    const calls = invokeSpy.mock.calls.filter(
      (c) => c[0] === "notification_log_append",
    );
    expect(calls).toHaveLength(1);
    const payload = calls[0][1] as { args: { kind: string } };
    expect(payload.args.kind).toBe("error");
  });

  it("a failing IPC must not throw out of pushToast", () => {
    invokeSpy.mockImplementation(async () => {
      throw new Error("simulated IPC failure");
    });
    const { result } = renderHook(() => useToasts());
    // Hook must not surface the rejection — log writes are advisory.
    act(() => result.current.pushToast("info", "still works"));
    expect(result.current.toasts).toHaveLength(1);
  });

  it("dedupe replacement still logs the new entry", () => {
    // Audit-style check: when a dedupeKey supersedes a prior toast,
    // the new toast still appears in the log. We don't suppress the
    // old entry's append (it already happened) — that's correct,
    // because the user did see both messages flash in the surface.
    const { result } = renderHook(() => useToasts());
    act(() =>
      result.current.pushToast(
        "info",
        "Switching…",
        undefined,
        { dedupeKey: "desktop-switch" },
      ),
    );
    act(() =>
      result.current.pushToast(
        "info",
        "Switched.",
        undefined,
        { dedupeKey: "desktop-switch" },
      ),
    );
    const calls = invokeSpy.mock.calls.filter(
      (c) => c[0] === "notification_log_append",
    );
    expect(calls).toHaveLength(2);
  });
});
