import { afterEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { USAGE_REFETCH_EVENT } from "../lib/events";

// Capture the (channel -> handler) pairs useUsage registers via
// useTauriEvent, so we can prove the usage::refetch listener exists and
// re-pulls usage when fired (the heal -> refetch reconcile).
const listeners: Record<string, () => void> = {};
vi.mock("./useTauriEvent", () => ({
  useTauriEvent: (channel: string | null, handler: () => void) => {
    if (channel) listeners[channel] = handler;
  },
}));

const fetchAllUsage = vi.fn(async () => ({}));
vi.mock("../api", () => ({
  api: {
    fetchAllUsage: () => fetchAllUsage(),
    refreshUsageFor: vi.fn(async () => ({})),
  },
}));

// useUsage emits `rebuild-tray-menu` on every fetch; stub it out.
vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn(async () => {}),
}));

import { useUsage } from "./useUsage";

/** Drain microtasks + one macrotask so `refreshUsage`'s `finally`
 *  (which clears the in-flight guard) runs before the next assertion. */
const flush = () =>
  act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });

afterEach(() => {
  for (const k of Object.keys(listeners)) delete listeners[k];
  vi.clearAllMocks();
});

describe("useUsage", () => {
  it("registers a usage::refetch listener that re-pulls usage when fired", async () => {
    renderHook(() => useUsage());
    await flush();

    // Mount triggers the initial fetch.
    expect(fetchAllUsage).toHaveBeenCalledTimes(1);
    // The heal -> refetch reconcile listener is wired to the shared
    // event constant.
    expect(listeners[USAGE_REFETCH_EVENT]).toBeTypeOf("function");

    // Firing it — as the backend orchestrator and verify paths do after
    // a heal — pulls a fresh snapshot.
    listeners[USAGE_REFETCH_EVENT]!();
    await flush();
    expect(fetchAllUsage).toHaveBeenCalledTimes(2);
  });
});
