import { afterEach, describe, expect, it, vi } from "vitest";
import { requestIdle, cancelIdle } from "./idle";

// jsdom doesn't implement rIC even though lib.dom types it as
// required — go through an index signature so install/delete in the
// test doesn't fight the DOM lib types.
const w = window as unknown as Record<string, unknown>;

afterEach(() => {
  delete w.requestIdleCallback;
  delete w.cancelIdleCallback;
  vi.useRealTimers();
});

describe("requestIdle / cancelIdle", () => {
  it("uses native requestIdleCallback when present", () => {
    const native = vi.fn().mockReturnValue(42);
    const nativeCancel = vi.fn();
    w.requestIdleCallback = native;
    w.cancelIdleCallback = nativeCancel;

    const cb = vi.fn();
    const handle = requestIdle(cb);
    expect(native).toHaveBeenCalledTimes(1);
    expect(handle).toBe(42);

    cancelIdle(handle);
    expect(nativeCancel).toHaveBeenCalledWith(42);
  });

  it("falls back to setTimeout(0) when rIC is missing", () => {
    vi.useFakeTimers();
    const cb = vi.fn();
    requestIdle(cb);
    expect(cb).not.toHaveBeenCalled();
    vi.advanceTimersByTime(0);
    expect(cb).toHaveBeenCalledTimes(1);
  });

  it("honors fallbackDelayMs in the setTimeout fallback", () => {
    vi.useFakeTimers();
    const cb = vi.fn();
    requestIdle(cb, { fallbackDelayMs: 250 });
    vi.advanceTimersByTime(249);
    expect(cb).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(cb).toHaveBeenCalledTimes(1);
  });

  it("cancelIdle clears the fallback timeout", () => {
    vi.useFakeTimers();
    const cb = vi.fn();
    const handle = requestIdle(cb, { fallbackDelayMs: 100 });
    cancelIdle(handle);
    vi.advanceTimersByTime(1_000);
    expect(cb).not.toHaveBeenCalled();
  });
});
