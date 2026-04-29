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

/**
 * Echo / `lastDismissed` contract — the slice the status bar reads
 * to replay the most recent toast text. Captured *after* the exit
 * animation completes so the live toast and its echo never coexist
 * (one signal per surface).
 */
describe("useToasts — lastDismissed echo", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("starts as null and only populates after a toast fully removes", () => {
    const { result } = renderHook(() => useToasts());
    expect(result.current.lastDismissed).toBeNull();

    act(() => result.current.pushToast("info", "saved"));
    // While the toast is on screen, the echo slot stays empty —
    // otherwise the status bar would render the same message twice.
    expect(result.current.lastDismissed).toBeNull();

    // 10 s auto-dismiss → 150 ms exit animation → final remove.
    act(() => vi.advanceTimersByTime(10_000));
    act(() => vi.advanceTimersByTime(200));
    expect(result.current.lastDismissed).toMatchObject({
      text: "saved",
      kind: "info",
    });
    expect(result.current.lastDismissed?.at).toBeTypeOf("number");
  });

  it("preserves the kind so the status bar can pick a tone", () => {
    // Error toasts are sticky, so we override durationMs to force
    // an auto-dismiss and exercise the kind-preservation path. The
    // status bar's echo segment renders error tone differently, so
    // the kind tag travelling through the dismissal pipeline still
    // matters even after the sticky-by-default change.
    const { result } = renderHook(() => useToasts());
    act(() =>
      result.current.pushToast("error", "oops", undefined, {
        durationMs: 10_000,
      }),
    );
    act(() => vi.advanceTimersByTime(10_200));
    expect(result.current.lastDismissed?.kind).toBe("error");
  });

  it("overwrites with the most recent dismissal", () => {
    const { result } = renderHook(() => useToasts());
    act(() => result.current.pushToast("info", "first", undefined, { durationMs: 500 }));
    act(() => vi.advanceTimersByTime(700));
    expect(result.current.lastDismissed?.text).toBe("first");

    act(() => result.current.pushToast("error", "second", undefined, { durationMs: 500 }));
    act(() => vi.advanceTimersByTime(700));
    expect(result.current.lastDismissed?.text).toBe("second");
    expect(result.current.lastDismissed?.kind).toBe("error");
  });

  it("clearLastDismissed empties the slot", () => {
    const { result } = renderHook(() => useToasts());
    act(() => result.current.pushToast("info", "saved", undefined, { durationMs: 100 }));
    act(() => vi.advanceTimersByTime(300));
    expect(result.current.lastDismissed).not.toBeNull();

    act(() => result.current.clearLastDismissed());
    expect(result.current.lastDismissed).toBeNull();
  });

  it("dedupe-cancellation does NOT echo the dropped toast (intentional)", () => {
    // Dedupe is the "user spammed an action" path — we cancel the
    // prior toast WITHOUT echoing it because the user almost certainly
    // didn't read the dropped message. Echoing every dedupe-stale
    // toast in the status bar would feel like ghosts of un-clicked
    // intent. The new toast still echoes normally when IT dismisses.
    const { result } = renderHook(() => useToasts());
    act(() =>
      result.current.pushToast("info", "first", undefined, {
        durationMs: 5000,
        dedupeKey: "k",
      }),
    );
    act(() =>
      result.current.pushToast("info", "second", undefined, {
        durationMs: 5000,
        dedupeKey: "k",
      }),
    );
    expect(result.current.lastDismissed).toBeNull();
    expect(result.current.toasts).toHaveLength(1);
    expect(result.current.toasts[0].text).toBe("second");
  });
});
