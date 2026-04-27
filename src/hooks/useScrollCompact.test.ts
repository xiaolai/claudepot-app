import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { act, renderHook } from "@testing-library/react";

import { useScrollCompact } from "./useScrollCompact";

/**
 * jsdom doesn't lay out elements, so it always reports
 * `scrollTop = 0` regardless of what we set. The hook only reads
 * `scrollTop` and listens for `scroll` events, so we mount a real
 * `<div>`, override `scrollTop` with a writable property, and
 * dispatch synthetic scroll events to drive the hook.
 *
 * Custom-property reads through `getComputedStyle` work in jsdom
 * for properties set via inline `style.setProperty` — we exercise
 * both the explicit-token path and the fallback default path.
 */

let host: HTMLDivElement;
let scrollTop = 0;

function makeScrollHost(opts?: { engage?: string; release?: string }) {
  host = document.createElement("div");
  if (opts?.engage !== undefined) {
    host.style.setProperty("--scroll-compact-engage", opts.engage);
  }
  if (opts?.release !== undefined) {
    host.style.setProperty("--scroll-compact-release", opts.release);
  }
  Object.defineProperty(host, "scrollTop", {
    configurable: true,
    get: () => scrollTop,
    set: (v: number) => {
      scrollTop = v;
    },
  });
  document.body.appendChild(host);
  return host;
}

function setScroll(top: number) {
  scrollTop = top;
  host.dispatchEvent(new Event("scroll"));
}

beforeEach(() => {
  scrollTop = 0;
});

afterEach(() => {
  document.body.innerHTML = "";
});

describe("useScrollCompact", () => {
  it("returns false at scrollTop 0", () => {
    const el = makeScrollHost({ engage: "16px", release: "4px" });
    const { result } = renderHook(() => useScrollCompact(el));
    expect(result.current).toBe(false);
  });

  it("engages once the user crosses the engage threshold", () => {
    const el = makeScrollHost({ engage: "16px", release: "4px" });
    const { result } = renderHook(() => useScrollCompact(el));
    act(() => setScroll(20));
    expect(result.current).toBe(true);
  });

  it("does NOT engage at a value below the engage threshold", () => {
    const el = makeScrollHost({ engage: "16px", release: "4px" });
    const { result } = renderHook(() => useScrollCompact(el));
    act(() => setScroll(10));
    expect(result.current).toBe(false);
  });

  it("stays compact in the hysteresis band (below engage but above release)", () => {
    const el = makeScrollHost({ engage: "16px", release: "4px" });
    const { result } = renderHook(() => useScrollCompact(el));
    act(() => setScroll(20));
    expect(result.current).toBe(true);
    act(() => setScroll(10));
    expect(result.current).toBe(true);
  });

  it("releases once the user scrolls back below the release threshold", () => {
    const el = makeScrollHost({ engage: "16px", release: "4px" });
    const { result } = renderHook(() => useScrollCompact(el));
    act(() => setScroll(20));
    act(() => setScroll(2));
    expect(result.current).toBe(false);
  });

  it("falls back to default thresholds when tokens are missing", () => {
    const el = makeScrollHost();
    const { result } = renderHook(() => useScrollCompact(el));
    act(() => setScroll(10));
    expect(result.current).toBe(false);
    act(() => setScroll(17));
    expect(result.current).toBe(true);
  });

  it("returns false and skips the listener when no element is provided", () => {
    const { result, rerender } = renderHook(
      ({ el }: { el: HTMLDivElement | null }) => useScrollCompact(el),
      { initialProps: { el: null } },
    );
    expect(result.current).toBe(false);
    rerender({ el: null });
    expect(result.current).toBe(false);
  });

  it("removes the scroll listener on unmount", () => {
    const el = makeScrollHost({ engage: "16px", release: "4px" });
    let removeCalls = 0;
    const realRemove = el.removeEventListener.bind(el);
    el.removeEventListener = ((...args: Parameters<typeof realRemove>) => {
      if (args[0] === "scroll") removeCalls += 1;
      return realRemove(...args);
    }) as typeof el.removeEventListener;
    const { unmount } = renderHook(() => useScrollCompact(el));
    unmount();
    expect(removeCalls).toBe(1);
  });
});
