import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook } from "@testing-library/react";
import type { ReactElement, ReactNode } from "react";

// Hoist mock for emit() — the dispatcher the migrated hook calls.
const emitMock = vi.fn();
vi.mock("../providers/AppStateProvider", () => ({
  useEmit: () => emitMock,
  // Tests don't render <AppStateProvider/>; useEmit returns the mock
  // directly so the renderHook wrapper can stay null.
  useAppState: () => ({}),
}));

const listenMock = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

import { useOpDoneNotifications } from "./useOpDoneNotifications";

type Listener = (ev: { payload?: unknown }) => void;

function setupTauriEventBus() {
  const listeners = new Map<string, Listener>();
  listenMock.mockImplementation(async (channel: string, fn: Listener) => {
    listeners.set(channel, fn);
    return () => listeners.delete(channel);
  });
  return {
    fire(channel: string, payload: unknown) {
      const fn = listeners.get(channel);
      if (fn) fn({ payload });
    },
    has(channel: string) {
      return listeners.has(channel);
    },
  };
}

describe("useOpDoneNotifications — emit() routing", () => {
  beforeEach(() => {
    emitMock.mockReset().mockResolvedValue({
      logId: 1,
      surfaces: {
        toast: false,
        osBanner: true,
        banner: false,
        log: true,
        ignoreFocus: false,
      },
      delivered: ["osBanner"],
    });
    listenMock.mockReset();
  });

  function wrapper({ children }: { children: ReactNode }) {
    return children as ReactElement;
  }

  it("subscribes to the cp-op-terminal channel", () => {
    setupTauriEventBus();
    renderHook(() => useOpDoneNotifications(), { wrapper });
    expect(listenMock).toHaveBeenCalledWith(
      "cp-op-terminal",
      expect.any(Function),
    );
  });

  it("fires emit() with category=opDoneUnfocused on terminal event", async () => {
    const bus = setupTauriEventBus();
    renderHook(() => useOpDoneNotifications(), { wrapper });
    // listen returns a promise that resolves after mount; wait a tick.
    await Promise.resolve();
    bus.fire("cp-op-terminal", {
      op_id: "op-1",
      kind: "verify_all",
      status: "complete",
      label: "Verified 3 accounts",
    });
    expect(emitMock).toHaveBeenCalledWith(
      expect.objectContaining({
        category: "opDoneUnfocused",
        title: "Verified 3 accounts",
        dedupeKey: "op:op-1",
      }),
    );
  });

  it("renders error status with kind=error and the error body", async () => {
    const bus = setupTauriEventBus();
    renderHook(() => useOpDoneNotifications(), { wrapper });
    await Promise.resolve();
    bus.fire("cp-op-terminal", {
      op_id: "op-2",
      kind: "session_prune",
      status: "error",
      label: "Pruning",
      error: "disk full",
    });
    expect(emitMock).toHaveBeenCalledWith(
      expect.objectContaining({
        category: "opDoneUnfocused",
        kind: "error",
        title: "Operation failed: Pruning",
        body: "disk full",
      }),
    );
  });

  it("truncates error bodies past 200 chars (matches legacy contract)", async () => {
    const bus = setupTauriEventBus();
    renderHook(() => useOpDoneNotifications(), { wrapper });
    await Promise.resolve();
    bus.fire("cp-op-terminal", {
      op_id: "op-3",
      kind: "x",
      status: "error",
      label: "X",
      error: "y".repeat(300),
    });
    const call = emitMock.mock.calls[0][0];
    expect(call.body.length).toBeLessThanOrEqual(200);
    expect(call.body.endsWith("…")).toBe(true);
  });

  it("does NOT buffer events — emit() reads prefs synchronously", async () => {
    // Audit issue #5 fix: legacy hook had a buffer that flushed once
    // on preferencesGet. After migration, emit() reads CategoryPrefs
    // from a cache that hydrates separately, so there's nothing to
    // buffer. Events dispatch immediately regardless of "pref-load"
    // state.
    const bus = setupTauriEventBus();
    renderHook(() => useOpDoneNotifications(), { wrapper });
    await Promise.resolve();
    bus.fire("cp-op-terminal", {
      op_id: "op-4",
      kind: "x",
      status: "complete",
      label: "Done",
    });
    expect(emitMock).toHaveBeenCalledTimes(1);
  });
});
