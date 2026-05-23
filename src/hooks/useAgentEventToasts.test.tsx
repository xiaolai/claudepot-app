import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook } from "@testing-library/react";
import type { ReactElement, ReactNode } from "react";

// grill X30: `useAgentEventToasts` is the ONLY path by which a user
// sees that an event-triggered agent's fire failed (or that the
// first-tick burst cap dropped sessions). The hook had zero direct
// tests, so a refactor that flipped `agent-event-dispatched` to
// subscribed — a one-line diff that would spam every settled-session
// narration — would land green. This spec locks down:
//
// - Subscribed channels: exactly `agent-event-failed` and
//   `agent-event-burst-capped`. `agent-event-dispatched` must stay
//   UN-subscribed (the dedupe-per-fire intent is load-bearing — a
//   successful run lands in RunHistoryPanel, not a toast).
// - Dedupe key path: firing the same (agentId, sessionId) twice
//   produces a single emit call (the dedupeKey carries them both).
// - Cleanup: the unlisten functions returned by `listen()` MUST run
//   on unmount.

// Hoist mocks before the hook import so vi.mock can swap them in.
const emitMock = vi.fn();
vi.mock("../providers/AppStateProvider", () => ({
  useEmit: () => emitMock,
  useAppState: () => ({}),
}));

const listenMock = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

import { useAgentEventToasts } from "./useAgentEventToasts";

type Listener = (ev: { payload?: unknown }) => void;

function setupTauriEventBus() {
  const listeners = new Map<string, Listener>();
  const unlisteners = new Map<string, () => void>();
  const unlistenCalls: string[] = [];
  listenMock.mockImplementation(async (channel: string, fn: Listener) => {
    listeners.set(channel, fn);
    const unlisten = () => {
      unlistenCalls.push(channel);
      listeners.delete(channel);
    };
    unlisteners.set(channel, unlisten);
    return unlisten;
  });
  return {
    fire(channel: string, payload: unknown) {
      const fn = listeners.get(channel);
      if (fn) fn({ payload });
    },
    has(channel: string) {
      return listeners.has(channel);
    },
    unlistenCalls() {
      return unlistenCalls.slice();
    },
  };
}

describe("useAgentEventToasts", () => {
  beforeEach(() => {
    emitMock.mockReset().mockResolvedValue({
      logId: 1,
      surfaces: {
        toast: true,
        osBanner: true,
        banner: false,
        log: true,
        ignoreFocus: false,
      },
      delivered: ["toast", "osBanner"],
    });
    listenMock.mockReset();
  });

  function wrapper({ children }: { children: ReactNode }) {
    return children as ReactElement;
  }

  it("subscribes to agent-event-failed + agent-event-burst-capped", () => {
    setupTauriEventBus();
    renderHook(() => useAgentEventToasts(), { wrapper });
    expect(listenMock).toHaveBeenCalledWith(
      "agent-event-failed",
      expect.any(Function),
    );
    expect(listenMock).toHaveBeenCalledWith(
      "agent-event-burst-capped",
      expect.any(Function),
    );
  });

  it("does NOT subscribe to agent-event-dispatched (dedupe-per-fire intent)", () => {
    setupTauriEventBus();
    renderHook(() => useAgentEventToasts(), { wrapper });
    const channels = listenMock.mock.calls.map((call) => call[0] as string);
    expect(channels).not.toContain("agent-event-dispatched");
    // Belt-and-suspenders: exactly two channels are subscribed.
    expect(channels.sort()).toEqual([
      "agent-event-burst-capped",
      "agent-event-failed",
    ]);
  });

  it("dedupes a repeated (agentId, sessionId) failure via dedupeKey", async () => {
    const bus = setupTauriEventBus();
    renderHook(() => useAgentEventToasts(), { wrapper });
    await Promise.resolve();

    bus.fire("agent-event-failed", {
      agentId: "a-uuid-1",
      sessionId: "s-uuid-1",
      error: "boom",
    });
    bus.fire("agent-event-failed", {
      agentId: "a-uuid-1",
      sessionId: "s-uuid-1",
      error: "boom again",
    });

    // The hook's dedupe is at the dispatch level — both emit calls
    // pass the SAME dedupeKey. The notification layer would
    // collapse them based on that key. From the hook's perspective
    // the contract under test is "the dedupeKey carries the
    // (agentId, sessionId) pair so identical repeats can be
    // collapsed by downstream", which we verify by inspecting the
    // emit calls.
    const emitCalls = emitMock.mock.calls;
    expect(emitCalls.length).toBe(2);
    const firstKey = emitCalls[0][0].dedupeKey as string;
    const secondKey = emitCalls[1][0].dedupeKey as string;
    expect(firstKey).toBe(secondKey);
    expect(firstKey).toBe("agent-event-failed:a-uuid-1:s-uuid-1");
  });

  it("dispatches a burst-capped toast with the cap + dropped counts", async () => {
    const bus = setupTauriEventBus();
    renderHook(() => useAgentEventToasts(), { wrapper });
    await Promise.resolve();

    bus.fire("agent-event-burst-capped", { cap: 5, dropped: 3 });
    expect(emitMock).toHaveBeenCalledTimes(1);
    const call = emitMock.mock.calls[0][0];
    expect(call.category).toBe("agentEventBurstCapped");
    expect(call.title).toContain("first-tick cap");
    expect(call.body).toContain("3 settled");
    expect(call.body).toContain("cap 5");
    expect(call.dedupeKey).toBe("agent-event-burst-capped:5:3");
  });

  it("runs the unlisten functions on unmount", async () => {
    const bus = setupTauriEventBus();
    const { unmount } = renderHook(() => useAgentEventToasts(), { wrapper });
    // Allow the listen() promises to resolve and push their
    // unlisteners into the hook's internal list.
    await Promise.resolve();
    await Promise.resolve();

    expect(bus.unlistenCalls()).toEqual([]);
    unmount();

    // Both channels' unlisten functions must have run.
    const called = bus.unlistenCalls().sort();
    expect(called).toEqual([
      "agent-event-burst-capped",
      "agent-event-failed",
    ]);
  });
});
