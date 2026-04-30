import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";

// Hoist mock factories so vi.mock() is allowed to reference them.
const sendNotificationMock = vi.fn();
const isPermissionGrantedMock = vi.fn();
const requestPermissionMock = vi.fn();

vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: (...args: unknown[]) => isPermissionGrantedMock(...args),
  requestPermission: (...args: unknown[]) => requestPermissionMock(...args),
  sendNotification: (...args: unknown[]) => sendNotificationMock(...args),
}));

const listenMock = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

// `api` is consumed for `preferencesGet`. Stub it to a controllable spy so
// the hook can flip `enabledRef` without going through Tauri.
const preferencesGetMock = vi.fn();
vi.mock("../api", () => ({
  api: {
    preferencesGet: () => preferencesGetMock(),
  },
}));

import { useOpDoneNotifications } from "./useOpDoneNotifications";
import { __resetForTests } from "../lib/notify";

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

describe("useOpDoneNotifications", () => {
  beforeEach(() => {
    sendNotificationMock.mockReset();
    isPermissionGrantedMock.mockReset().mockResolvedValue(true);
    requestPermissionMock.mockReset().mockResolvedValue("granted");
    preferencesGetMock.mockReset();
    listenMock.mockReset();
    __resetForTests();
    // Default: window is unfocused so the focus gate is permissive.
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("does nothing when notify_on_op_done is false", async () => {
    preferencesGetMock.mockResolvedValue({
      notify_on_op_done: false,
      notify_on_error: false,
      notify_on_idle_done: false,
      notify_on_stuck_minutes: null,
      notify_on_waiting: false,
    });
    const bus = setupTauriEventBus();
    renderHook(() => useOpDoneNotifications());
    await waitFor(() => expect(bus.has("cp-op-terminal")).toBe(true));

    act(() => {
      bus.fire("cp-op-terminal", {
        op_id: "op-1",
        kind: "verify_all",
        status: "complete",
        label: "Verified 4 accounts",
      });
    });

    expect(sendNotificationMock).not.toHaveBeenCalled();
  });

  it("fires sendNotification on a complete event when enabled", async () => {
    preferencesGetMock.mockResolvedValue({
      notify_on_op_done: true,
      notify_on_error: false,
      notify_on_idle_done: false,
      notify_on_stuck_minutes: null,
      notify_on_waiting: false,
    });
    const bus = setupTauriEventBus();
    renderHook(() => useOpDoneNotifications());

    // Wait both for the listener to be wired AND for the
    // preferencesGet promise to flush — `enabledRef` flips inside the
    // `.then` callback, which resolves on the next microtask.
    await waitFor(() => expect(bus.has("cp-op-terminal")).toBe(true));
    await waitFor(() => expect(preferencesGetMock).toHaveBeenCalled());
    // Allow microtasks to run so applyPrefs() has settled.
    await Promise.resolve();
    await Promise.resolve();

    act(() => {
      bus.fire("cp-op-terminal", {
        op_id: "op-1",
        kind: "verify_all",
        status: "complete",
        label: "Verified 4 accounts",
      });
    });

    await waitFor(() => expect(sendNotificationMock).toHaveBeenCalled());
    expect(sendNotificationMock).toHaveBeenCalledWith({
      title: "Verified 4 accounts",
      body: "Done.",
      group: "op:verify_all",
      sound: "default",
    });
  });

  it("includes error detail in the body on a failed terminal event", async () => {
    preferencesGetMock.mockResolvedValue({
      notify_on_op_done: true,
      notify_on_error: false,
      notify_on_idle_done: false,
      notify_on_stuck_minutes: null,
      notify_on_waiting: false,
    });
    const bus = setupTauriEventBus();
    renderHook(() => useOpDoneNotifications());

    await waitFor(() => expect(bus.has("cp-op-terminal")).toBe(true));
    await waitFor(() => expect(preferencesGetMock).toHaveBeenCalled());
    await Promise.resolve();
    await Promise.resolve();

    act(() => {
      bus.fire("cp-op-terminal", {
        op_id: "op-2",
        kind: "move_project",
        status: "error",
        label: "Renamed proj-a → proj-b",
        error: "filesystem busy",
      });
    });

    await waitFor(() => expect(sendNotificationMock).toHaveBeenCalled());
    expect(sendNotificationMock).toHaveBeenCalledWith({
      title: "Operation failed: Renamed proj-a → proj-b",
      body: "filesystem busy",
      group: "op:move_project",
      sound: "default",
    });
  });

  it("buffers terminal events that arrive before prefsGet resolves and flushes them once enabled", async () => {
    // Reproduce the startup race: the cp-op-terminal listener attaches
    // before the initial preferencesGet() round-trip lands. A user
    // triggering an op-end in those first few hundred ms used to be
    // dropped because enabledRef defaulted to false. The buffer must
    // hold until prefs say "enabled = true", then flush.
    let resolvePrefs: (p: unknown) => void = () => {};
    preferencesGetMock.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolvePrefs = resolve;
        }),
    );
    const bus = setupTauriEventBus();
    renderHook(() => useOpDoneNotifications());

    await waitFor(() => expect(bus.has("cp-op-terminal")).toBe(true));
    // Fire BEFORE prefs resolve.
    act(() => {
      bus.fire("cp-op-terminal", {
        op_id: "op-buffered",
        kind: "verify_all",
        status: "complete",
        label: "Verified accounts",
      });
    });
    // Nothing yet — buffered.
    expect(sendNotificationMock).not.toHaveBeenCalled();

    // Now prefs resolve with enabled = true. Buffer must flush.
    act(() => {
      resolvePrefs({
        notify_on_op_done: true,
        notify_on_error: false,
        notify_on_idle_done: false,
        notify_on_stuck_minutes: null,
      });
    });
    await waitFor(() => expect(sendNotificationMock).toHaveBeenCalled());
    expect(sendNotificationMock).toHaveBeenCalledWith(
      expect.objectContaining({
        title: "Verified accounts",
        body: "Done.",
      }),
    );
  });

  it("drops buffered events when prefs load with toggle off", async () => {
    let resolvePrefs: (p: unknown) => void = () => {};
    preferencesGetMock.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolvePrefs = resolve;
        }),
    );
    const bus = setupTauriEventBus();
    renderHook(() => useOpDoneNotifications());

    await waitFor(() => expect(bus.has("cp-op-terminal")).toBe(true));
    act(() => {
      bus.fire("cp-op-terminal", {
        op_id: "op-buffered-off",
        kind: "verify_all",
        status: "complete",
        label: "Verified accounts",
      });
    });
    act(() => {
      resolvePrefs({
        notify_on_op_done: false,
        notify_on_error: false,
        notify_on_idle_done: false,
        notify_on_stuck_minutes: null,
      });
    });
    // Drain microtasks; nothing should fire.
    await Promise.resolve();
    await Promise.resolve();
    expect(sendNotificationMock).not.toHaveBeenCalled();
  });

  it("suppresses dispatch when the window is focused", async () => {
    (document.hasFocus as ReturnType<typeof vi.fn>).mockReturnValue(true);
    preferencesGetMock.mockResolvedValue({
      notify_on_op_done: true,
      notify_on_error: false,
      notify_on_idle_done: false,
      notify_on_stuck_minutes: null,
      notify_on_waiting: false,
    });
    const bus = setupTauriEventBus();
    renderHook(() => useOpDoneNotifications());

    await waitFor(() => expect(bus.has("cp-op-terminal")).toBe(true));
    await waitFor(() => expect(preferencesGetMock).toHaveBeenCalled());
    await Promise.resolve();
    await Promise.resolve();

    act(() => {
      bus.fire("cp-op-terminal", {
        op_id: "op-3",
        kind: "verify_all",
        status: "complete",
        label: "Verified accounts",
      });
    });

    // Focus gate suppresses sendNotification but the listener still
    // fires; assert no OS dispatch happened.
    await Promise.resolve();
    await Promise.resolve();
    expect(sendNotificationMock).not.toHaveBeenCalled();
  });
});
