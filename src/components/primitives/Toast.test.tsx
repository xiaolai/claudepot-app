import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { render, act, cleanup } from "@testing-library/react";

import { Toast } from "./Toast";

describe("Toast primitive — auto-dismiss", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    cleanup();
  });

  it("fires onDismiss after the default 10 000 ms", () => {
    const onDismiss = vi.fn();
    render(<Toast message="hi" onDismiss={onDismiss} />);
    // Just before the window expires: still silent.
    act(() => vi.advanceTimersByTime(9_999));
    expect(onDismiss).not.toHaveBeenCalled();
    // One ms past: fires exactly once.
    act(() => vi.advanceTimersByTime(1));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("respects a shorter durationMs override", () => {
    const onDismiss = vi.fn();
    render(
      <Toast message="hi" onDismiss={onDismiss} durationMs={1000} />,
    );
    act(() => vi.advanceTimersByTime(1000));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("Infinity disables auto-dismiss (sticky)", () => {
    const onDismiss = vi.fn();
    render(
      <Toast message="hi" onDismiss={onDismiss} durationMs={Infinity} />,
    );
    act(() => vi.advanceTimersByTime(60_000));
    expect(onDismiss).not.toHaveBeenCalled();
  });

  it("null durationMs disables auto-dismiss", () => {
    const onDismiss = vi.fn();
    render(
      <Toast message="hi" onDismiss={onDismiss} durationMs={null} />,
    );
    act(() => vi.advanceTimersByTime(60_000));
    expect(onDismiss).not.toHaveBeenCalled();
  });

  it("does not reset the timer when the parent re-renders with the same message", () => {
    // Regression guard: callers typically pass `onDismiss={() =>
    // setToast(null)}` which is a new function identity on every
    // parent render. If the effect depended on `onDismiss`, unrelated
    // re-renders would reset the timer and the toast would never
    // auto-close.
    const onDismiss = vi.fn();
    const { rerender } = render(<Toast message="hi" onDismiss={onDismiss} />);
    act(() => vi.advanceTimersByTime(5000));
    // Simulate an unrelated parent re-render: new arrow, same message.
    rerender(<Toast message="hi" onDismiss={() => onDismiss()} />);
    act(() => vi.advanceTimersByTime(5000));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("resets the clock when the message changes", () => {
    const onDismiss = vi.fn();
    const { rerender } = render(
      <Toast message="first" onDismiss={onDismiss} durationMs={1000} />,
    );
    act(() => vi.advanceTimersByTime(500));
    // Swap to a new message before the first auto-dismisses.
    rerender(
      <Toast message="second" onDismiss={onDismiss} durationMs={1000} />,
    );
    // 500 ms elapsed since the new message — should still be silent.
    act(() => vi.advanceTimersByTime(500));
    expect(onDismiss).not.toHaveBeenCalled();
    // Another 500 ms completes the second toast's window.
    act(() => vi.advanceTimersByTime(500));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("does not fire when message is null", () => {
    const onDismiss = vi.fn();
    render(<Toast message={null} onDismiss={onDismiss} />);
    act(() => vi.advanceTimersByTime(30_000));
    expect(onDismiss).not.toHaveBeenCalled();
  });

  it("clears the timer on unmount", () => {
    const onDismiss = vi.fn();
    const { unmount } = render(<Toast message="hi" onDismiss={onDismiss} />);
    act(() => vi.advanceTimersByTime(5000));
    unmount();
    act(() => vi.advanceTimersByTime(30_000));
    expect(onDismiss).not.toHaveBeenCalled();
  });
});
