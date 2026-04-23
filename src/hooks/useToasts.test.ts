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

  it("auto-dismisses error toasts after 10 000 ms (previously sticky)", () => {
    // Regression guard: before this change errors had no timer, which
    // let a transient error linger across unrelated navigation.
    const { result } = renderHook(() => useToasts());
    act(() => result.current.pushToast("error", "oops"));
    expect(result.current.toasts).toHaveLength(1);

    act(() => vi.advanceTimersByTime(10_000));
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
