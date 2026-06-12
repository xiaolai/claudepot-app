import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook } from "@testing-library/react";

const listenMock = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

const syncFromCurrentCc = vi.fn();
const verifyAllAccounts = vi.fn();
vi.mock("../api", () => ({
  api: {
    syncFromCurrentCc: (...a: unknown[]) => syncFromCurrentCc(...a),
    verifyAllAccounts: (...a: unknown[]) => verifyAllAccounts(...a),
  },
}));

import { useAppMenuRouter } from "./useAppMenuRouter";

type Listener = (ev: { payload?: unknown }) => void;

function setupBus() {
  const listeners = new Map<string, Listener>();
  listenMock.mockImplementation(async (channel: string, fn: Listener) => {
    listeners.set(channel, fn);
    return () => listeners.delete(channel);
  });
  return {
    fire: (channel: string, payload: unknown) =>
      listeners.get(channel)?.({ payload }),
  };
}

function renderRouter() {
  const args = {
    setSection: vi.fn(),
    toggleTheme: vi.fn(),
    refreshAccounts: vi.fn().mockResolvedValue(undefined),
    pushToast: vi.fn(),
  };
  const utils = renderHook(() => useAppMenuRouter(args));
  return { args, ...utils };
}

beforeEach(() => {
  listenMock.mockReset();
  syncFromCurrentCc.mockReset();
  verifyAllAccounts.mockReset();
  try {
    sessionStorage.clear();
  } catch {
    /* ignore */
  }
});

describe("useAppMenuRouter", () => {
  it("subscribes to the app-menu channel exactly once", async () => {
    setupBus();
    renderRouter();
    await Promise.resolve();
    expect(listenMock).toHaveBeenCalledTimes(1);
    expect(listenMock).toHaveBeenCalledWith("app-menu", expect.any(Function));
  });

  it("routes app-menu:nav:<section> to setSection", async () => {
    const bus = setupBus();
    const { args } = renderRouter();
    await Promise.resolve();

    bus.fire("app-menu", "app-menu:nav:projects");
    expect(args.setSection).toHaveBeenCalledWith("projects");
  });

  it("ignores nav commands with unknown section ids", async () => {
    const bus = setupBus();
    const { args } = renderRouter();
    await Promise.resolve();

    bus.fire("app-menu", "app-menu:nav:not-a-section");
    expect(args.setSection).not.toHaveBeenCalled();
  });

  it("routes the settings subtab form through the deep-link store", async () => {
    const bus = setupBus();
    const { args } = renderRouter();
    await Promise.resolve();

    bus.fire("app-menu", "app-menu:nav:settings:health");
    expect(args.setSection).toHaveBeenCalledWith("settings");
    // triggerSettingsTab persists the cold-mount deep link.
    expect(sessionStorage.getItem("claudepot.deepLink.settingsTab")).toBe(
      "health",
    );
  });

  it("toggles the theme on app-menu:view:toggle-theme", async () => {
    const bus = setupBus();
    const { args } = renderRouter();
    await Promise.resolve();

    bus.fire("app-menu", "app-menu:view:toggle-theme");
    expect(args.toggleTheme).toHaveBeenCalledTimes(1);
  });

  it("syncs from CC and toasts the result", async () => {
    const bus = setupBus();
    syncFromCurrentCc.mockResolvedValue("a@example.com");
    const { args } = renderRouter();
    await Promise.resolve();

    bus.fire("app-menu", "app-menu:account:sync-cc");
    await Promise.resolve();
    await Promise.resolve();
    expect(args.pushToast).toHaveBeenCalledWith(
      "info",
      "Synced a@example.com from CC.",
    );
  });

  it("verify-all toasts then refreshes accounts", async () => {
    const bus = setupBus();
    verifyAllAccounts.mockResolvedValue(undefined);
    const { args } = renderRouter();
    await Promise.resolve();

    bus.fire("app-menu", "app-menu:account:verify-all");
    await Promise.resolve();
    await Promise.resolve();
    expect(args.pushToast).toHaveBeenCalledWith("info", "Verify all complete.");
    expect(args.refreshAccounts).toHaveBeenCalled();
  });
});
