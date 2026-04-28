/**
 * UpdateProvider unit tests. We exercise the pure scheduler
 * (`shouldCheckNow`) directly and the provider's public API via
 * `renderHook`. The actual `@tauri-apps/plugin-updater` is mocked at
 * the module level so the hook can drive the state machine without
 * a webview present.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";

import { shouldCheckNow, UpdateProvider, useUpdater } from "./UpdateProvider";

const checkMock = vi.fn();
const updaterSupportedMock = vi.fn().mockResolvedValue(true);

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: (...args: unknown[]) => checkMock(...args),
}));

vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: vi.fn().mockResolvedValue(undefined),
}));

// The provider probes platform support via the Tauri-bound API on
// mount. Stub the whole `api` surface so the mock survives across
// tests; tests can override `updaterSupportedMock` per case.
vi.mock("../api", () => ({
  api: {
    updaterSupported: () => updaterSupportedMock(),
  },
}));

const wrapper = ({ children }: { children: ReactNode }) => (
  <UpdateProvider>{children}</UpdateProvider>
);

describe("shouldCheckNow", () => {
  it("returns false when auto-check is disabled", () => {
    expect(shouldCheckNow(false, "startup", null)).toBe(false);
    expect(shouldCheckNow(false, "daily", null)).toBe(false);
  });

  it("returns false for manual frequency", () => {
    expect(shouldCheckNow(true, "manual", null)).toBe(false);
    expect(shouldCheckNow(true, "manual", Date.now())).toBe(false);
  });

  it("returns true for startup frequency, regardless of last-checked", () => {
    expect(shouldCheckNow(true, "startup", null)).toBe(true);
    expect(shouldCheckNow(true, "startup", Date.now())).toBe(true);
  });

  it("returns true for daily when last-checked is older than 24h", () => {
    const now = Date.now();
    const ONE_DAY = 24 * 60 * 60 * 1000;
    expect(shouldCheckNow(true, "daily", now - ONE_DAY - 1)).toBe(true);
    expect(shouldCheckNow(true, "daily", now - ONE_DAY + 1000)).toBe(false);
  });

  it("returns true for weekly when last-checked is older than 7d", () => {
    const now = Date.now();
    const ONE_WEEK = 7 * 24 * 60 * 60 * 1000;
    expect(shouldCheckNow(true, "weekly", now - ONE_WEEK - 1)).toBe(true);
    expect(shouldCheckNow(true, "weekly", now - ONE_WEEK + 1000)).toBe(false);
  });

  it("returns true on first run (no last-checked timestamp)", () => {
    expect(shouldCheckNow(true, "daily", null)).toBe(true);
    expect(shouldCheckNow(true, "weekly", null)).toBe(true);
  });
});

describe("UpdateProvider — checkNow lifecycle", () => {
  beforeEach(() => {
    checkMock.mockReset();
    updaterSupportedMock.mockReset();
    updaterSupportedMock.mockResolvedValue(true);
    try {
      localStorage.clear();
    } catch {
      // jsdom storage may not be present in some envs.
    }
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("reports supported=true after the probe lands", async () => {
    const { result } = renderHook(() => useUpdater(), { wrapper });
    expect(result.current.supported).toBeNull();
    await waitFor(() => expect(result.current.supported).toBe(true));
  });

  it("reports supported=false on Linux .deb (probe returns false)", async () => {
    updaterSupportedMock.mockResolvedValue(false);
    const { result } = renderHook(() => useUpdater(), { wrapper });
    await waitFor(() => expect(result.current.supported).toBe(false));
  });

  it("transitions to 'up-to-date' when no update is returned", async () => {
    checkMock.mockResolvedValue(null);
    const { result } = renderHook(() => useUpdater(), { wrapper });

    await act(async () => {
      await result.current.checkNow();
    });
    expect(result.current.status).toBe("up-to-date");
    expect(result.current.updateInfo).toBeNull();
    expect(result.current.lastCheckedAt).not.toBeNull();
  });

  it("transitions to 'available' and exposes UpdateInfo when an update is returned", async () => {
    checkMock.mockResolvedValue({
      version: "0.0.7",
      currentVersion: "0.0.6",
      body: "release notes",
      date: "2026-04-28T10:00:00Z",
      downloadAndInstall: vi.fn(),
      close: vi.fn().mockResolvedValue(undefined),
    });
    const { result } = renderHook(() => useUpdater(), { wrapper });

    await act(async () => {
      await result.current.checkNow();
    });
    expect(result.current.status).toBe("available");
    expect(result.current.updateInfo).toEqual({
      version: "0.0.7",
      currentVersion: "0.0.6",
      notes: "release notes",
      pubDate: "2026-04-28T10:00:00Z",
    });
  });

  it("transitions to 'error' and surfaces the message when check throws", async () => {
    checkMock.mockRejectedValue(new Error("network down"));
    const { result } = renderHook(() => useUpdater(), { wrapper });

    await act(async () => {
      await result.current.checkNow();
    });
    expect(result.current.status).toBe("error");
    expect(result.current.error).toBe("network down");
  });

  it("skipThisVersion sets isSkipped, resetSkip clears it", async () => {
    checkMock.mockResolvedValue({
      version: "0.0.7",
      currentVersion: "0.0.6",
      body: "",
      date: null,
      downloadAndInstall: vi.fn(),
      close: vi.fn().mockResolvedValue(undefined),
    });
    const { result } = renderHook(() => useUpdater(), { wrapper });

    await act(async () => {
      await result.current.checkNow();
    });
    expect(result.current.isSkipped).toBe(false);

    act(() => result.current.skipThisVersion());
    expect(result.current.isSkipped).toBe(true);

    act(() => result.current.resetSkip());
    expect(result.current.isSkipped).toBe(false);
  });

  it("setAutoCheckEnabled persists to localStorage", () => {
    const { result } = renderHook(() => useUpdater(), { wrapper });
    expect(result.current.autoCheckEnabled).toBe(true);

    act(() => result.current.setAutoCheckEnabled(false));
    expect(result.current.autoCheckEnabled).toBe(false);
    expect(localStorage.getItem("claudepot.update.autoCheckEnabled")).toBe(
      "false",
    );
  });

  it("setCheckFrequency persists to localStorage", () => {
    const { result } = renderHook(() => useUpdater(), { wrapper });
    act(() => result.current.setCheckFrequency("weekly"));
    expect(result.current.checkFrequency).toBe("weekly");
    expect(localStorage.getItem("claudepot.update.checkFrequency")).toBe(
      "weekly",
    );
  });
});
