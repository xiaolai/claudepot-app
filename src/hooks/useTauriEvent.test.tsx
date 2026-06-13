import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook } from "@testing-library/react";
import { act } from "react";

// Hoist the listen mock before the hook import.
const listenMock = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

import { useTauriEvent, useTauriEvents } from "./useTauriEvent";

type Listener = (ev: { payload?: unknown }) => void;

function setupTauriEventBus() {
  const listeners = new Map<string, Listener>();
  const unlistenCalls: string[] = [];
  listenMock.mockImplementation(async (channel: string, fn: Listener) => {
    listeners.set(channel, fn);
    return () => {
      unlistenCalls.push(channel);
      listeners.delete(channel);
    };
  });
  return {
    fire(channel: string, payload: unknown) {
      listeners.get(channel)?.({ payload });
    },
    has(channel: string) {
      return listeners.has(channel);
    },
    unlistenCalls: () => unlistenCalls.slice(),
  };
}

beforeEach(() => {
  listenMock.mockReset();
});

describe("useTauriEvent", () => {
  it("subscribes and delivers events to the handler", async () => {
    const bus = setupTauriEventBus();
    const handler = vi.fn();
    renderHook(() => useTauriEvent<string>("chan-a", handler));
    await act(async () => {});

    bus.fire("chan-a", "hello");
    expect(handler).toHaveBeenCalledTimes(1);
    expect(handler.mock.calls[0][0]).toMatchObject({ payload: "hello" });
  });

  it("skips subscription when channel is null", async () => {
    setupTauriEventBus();
    renderHook(() => useTauriEvent(null, vi.fn()));
    await act(async () => {});
    expect(listenMock).not.toHaveBeenCalled();
  });

  it("does NOT resubscribe when the handler identity changes", async () => {
    const bus = setupTauriEventBus();
    const first = vi.fn();
    const second = vi.fn();
    const { rerender } = renderHook(
      ({ h }: { h: (ev: unknown) => void }) =>
        useTauriEvent<string>("chan-a", h),
      { initialProps: { h: first } },
    );
    await act(async () => {});
    expect(listenMock).toHaveBeenCalledTimes(1);

    rerender({ h: second });
    await act(async () => {});
    // Still one subscription — no unlisten/listen churn.
    expect(listenMock).toHaveBeenCalledTimes(1);
    expect(bus.unlistenCalls()).toEqual([]);

    // ...and the LATEST handler receives the event.
    bus.fire("chan-a", "x");
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);
  });

  it("resubscribes when the channel changes", async () => {
    const bus = setupTauriEventBus();
    const handler = vi.fn();
    const { rerender } = renderHook(
      ({ c }: { c: string }) => useTauriEvent(c, handler),
      { initialProps: { c: "chan-a" } },
    );
    await act(async () => {});
    rerender({ c: "chan-b" });
    await act(async () => {});

    expect(bus.unlistenCalls()).toEqual(["chan-a"]);
    expect(bus.has("chan-b")).toBe(true);
  });

  it("unlistens on unmount", async () => {
    const bus = setupTauriEventBus();
    const { unmount } = renderHook(() => useTauriEvent("chan-a", vi.fn()));
    await act(async () => {});
    unmount();
    expect(bus.unlistenCalls()).toEqual(["chan-a"]);
  });

  it("drops the listener immediately when unmounted before listen() resolves", async () => {
    const unlisten = vi.fn();
    let resolveListen: (fn: () => void) => void = () => {};
    listenMock.mockImplementation(
      () =>
        new Promise<() => void>((resolve) => {
          resolveListen = resolve;
        }),
    );
    const { unmount } = renderHook(() => useTauriEvent("chan-a", vi.fn()));
    unmount();
    await act(async () => {
      resolveListen(unlisten);
    });
    expect(unlisten).toHaveBeenCalledTimes(1);
  });
});

describe("useTauriEvents", () => {
  it("subscribes one listener per channel and routes by channel", async () => {
    const bus = setupTauriEventBus();
    const onA = vi.fn();
    const onB = vi.fn();
    renderHook(() =>
      useTauriEvents({
        "chan-a": onA as never,
        "chan-b": onB as never,
      }),
    );
    await act(async () => {});

    expect(bus.has("chan-a")).toBe(true);
    expect(bus.has("chan-b")).toBe(true);

    bus.fire("chan-a", { v: 1 });
    expect(onA).toHaveBeenCalledTimes(1);
    expect(onB).not.toHaveBeenCalled();
    bus.fire("chan-b", { v: 2 });
    expect(onB).toHaveBeenCalledTimes(1);
  });

  it("does NOT resubscribe when handler identities change (same channels)", async () => {
    const bus = setupTauriEventBus();
    const second = vi.fn();
    const { rerender } = renderHook(
      ({ h }: { h: Record<string, never> }) => useTauriEvents(h),
      {
        initialProps: {
          h: { "chan-a": vi.fn() as never } as Record<string, never>,
        },
      },
    );
    await act(async () => {});
    expect(listenMock).toHaveBeenCalledTimes(1);

    rerender({ h: { "chan-a": second as never } as Record<string, never> });
    await act(async () => {});
    expect(listenMock).toHaveBeenCalledTimes(1);
    expect(bus.unlistenCalls()).toEqual([]);

    bus.fire("chan-a", {});
    expect(second).toHaveBeenCalledTimes(1);
  });

  it("tears down every channel on unmount", async () => {
    const bus = setupTauriEventBus();
    const { unmount } = renderHook(() =>
      useTauriEvents({
        "chan-a": vi.fn() as never,
        "chan-b": vi.fn() as never,
      }),
    );
    await act(async () => {});
    unmount();
    expect(bus.unlistenCalls().sort()).toEqual(["chan-a", "chan-b"]);
  });

  it("skips subscription entirely for null", async () => {
    setupTauriEventBus();
    renderHook(() => useTauriEvents(null));
    await act(async () => {});
    expect(listenMock).not.toHaveBeenCalled();
  });
});
