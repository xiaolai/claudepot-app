import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act } from "@testing-library/react";

import { useToasts } from "./useToasts";

describe("useToasts — auto-dismiss policy", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("auto-dismisses info toasts after 10 000 ms (default)", () => {
    const { result } = renderHook(() => useToasts());
    act(() => result.current.pushToast("info", "saved"));
    expect(result.current.toasts).toHaveLength(1);

    act(() => vi.advanceTimersByTime(9_999));
    expect(result.current.toasts).toHaveLength(1);

    act(() => vi.advanceTimersByTime(1));
    // Mid-exit: the toast is marked `exiting` and removed after the
    // 150 ms exit animation.
    act(() => vi.advanceTimersByTime(200));
    expect(result.current.toasts).toHaveLength(0);
  });

  it("error toasts are sticky by default", () => {
    // Errors carry diagnostic copy worth screenshotting / dictating —
    // auto-dismiss is the wrong default when the message IS the
    // diagnostic. The toast still has a close button + dedupeKey;
    // accidental accumulation is bounded by user dismissal. Callers
    // can override with an explicit `durationMs` for transient
    // errors that don't need to persist.
    const { result } = renderHook(() => useToasts());
    act(() => result.current.pushToast("error", "oops"));
    expect(result.current.toasts).toHaveLength(1);

    // 60 s well past the 10 s info default — error stays.
    act(() => vi.advanceTimersByTime(60_000));
    expect(result.current.toasts).toHaveLength(1);
  });

  it("explicit durationMs overrides the sticky default for errors", () => {
    // Regression guard for the override path: callers that DO want a
    // transient error toast can still pass a finite duration.
    const { result } = renderHook(() => useToasts());
    act(() =>
      result.current.pushToast("error", "transient", undefined, {
        durationMs: 1_000,
      }),
    );
    act(() => vi.advanceTimersByTime(1_000));
    act(() => vi.advanceTimersByTime(200));
    expect(result.current.toasts).toHaveLength(0);
  });

  it("respects the undoMs window for undo toasts (default 3 000 ms)", () => {
    const onUndo = vi.fn();
    const { result } = renderHook(() => useToasts());
    act(() => result.current.pushToast("info", "switched", onUndo));
    act(() => vi.advanceTimersByTime(3_000));
    act(() => vi.advanceTimersByTime(200));
    expect(result.current.toasts).toHaveLength(0);
  });

  it("runs onCommit iff the undo window elapses without user action", () => {
    const onUndo = vi.fn();
    const onCommit = vi.fn();
    const { result } = renderHook(() => useToasts());
    act(() =>
      result.current.pushToast("info", "will commit", onUndo, {
        undoMs: 1000,
        onCommit,
      }),
    );
    act(() => vi.advanceTimersByTime(1000));
    expect(onCommit).toHaveBeenCalledTimes(1);
    expect(onUndo).not.toHaveBeenCalled();
  });

  it("durationMs: Infinity keeps a toast sticky", () => {
    const { result } = renderHook(() => useToasts());
    act(() =>
      result.current.pushToast("info", "persistent", undefined, {
        durationMs: Infinity,
      }),
    );
    act(() => vi.advanceTimersByTime(60_000));
    expect(result.current.toasts).toHaveLength(1);
  });

  it("durationMs override applies to both info and error toasts", () => {
    const { result } = renderHook(() => useToasts());
    act(() =>
      result.current.pushToast("error", "short error", undefined, {
        durationMs: 500,
      }),
    );
    act(() => vi.advanceTimersByTime(500));
    act(() => vi.advanceTimersByTime(200));
    expect(result.current.toasts).toHaveLength(0);
  });

  it("manual dismiss (X) commits the deferred action", () => {
    // Audit F1 regression: closing "Switching Desktop to X…" with the
    // X button used to silently cancel the switch — the toast said the
    // action was happening, then nothing happened. A dismiss that
    // isn't an Undo must commit.
    const onCommit = vi.fn();
    const { result } = renderHook(() => useToasts());
    act(() =>
      result.current.pushToast("info", "switching…", () => {}, {
        undoMs: 3000,
        onCommit,
      }),
    );
    const id = result.current.toasts[0].id;
    act(() => result.current.dismissToast(id));
    expect(onCommit).toHaveBeenCalledTimes(1);

    // The cleared auto-timer + run-once map must not double-commit.
    act(() => vi.advanceTimersByTime(5000));
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it("undo dismiss (skipCommit) does NOT commit", () => {
    const onCommit = vi.fn();
    const onUndo = vi.fn();
    const { result } = renderHook(() => useToasts());
    act(() =>
      result.current.pushToast("info", "switching…", onUndo, {
        undoMs: 3000,
        onCommit,
      }),
    );
    const id = result.current.toasts[0].id;
    // Mirrors the ToastContainer Undo button: onUndo() then a
    // skipCommit dismiss.
    act(() => {
      result.current.toasts[0].onUndo?.();
      result.current.dismissToast(id, { skipCommit: true });
    });
    expect(onUndo).toHaveBeenCalledTimes(1);
    expect(onCommit).not.toHaveBeenCalled();

    act(() => vi.advanceTimersByTime(5000));
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("dedupeKey cancels the prior toast's timer before replacing it", () => {
    // Regression guard: without the timer clear on dedupe, rapid-fire
    // actions would both commit because two parallel timers were still
    // running.
    const commitA = vi.fn();
    const commitB = vi.fn();
    const { result } = renderHook(() => useToasts());
    act(() =>
      result.current.pushToast("info", "A", () => {}, {
        undoMs: 1000,
        onCommit: commitA,
        dedupeKey: "swap",
      }),
    );
    act(() =>
      result.current.pushToast("info", "B", () => {}, {
        undoMs: 1000,
        onCommit: commitB,
        dedupeKey: "swap",
      }),
    );
    act(() => vi.advanceTimersByTime(1000));
    expect(commitA).not.toHaveBeenCalled();
    expect(commitB).toHaveBeenCalledTimes(1);
  });
});

