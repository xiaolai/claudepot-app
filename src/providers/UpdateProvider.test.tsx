/**
 * UpdateProvider unit tests. We exercise the pure scheduler
 * (`shouldCheckNow`) directly and the provider's public API via
 * `renderHook`.
 *
 * Channel-aware rewire: the provider now drives the Rust `release_*`
 * commands (`releaseUpdateCheck` / `releaseUpdateInstall` /
 * `releaseChannelGet` / `releaseChannelSet`) instead of the JS
 * `@tauri-apps/plugin-updater`. We mock the whole `api` surface so
 * the hook can drive the state machine without a webview.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import {
  renderHook,
  act,
  waitFor,
  screen,
  fireEvent,
} from "@testing-library/react";
import type { ReactNode } from "react";

import { shouldCheckNow, UpdateProvider, useUpdater } from "./UpdateProvider";

const releaseUpdateCheckMock = vi.fn();
const releaseUpdateInstallMock = vi.fn();
const releaseChannelGetMock = vi.fn();
const releaseChannelSetMock = vi.fn();
const relaunchBusyOpsMock = vi.fn();
const updaterSupportedMock = vi.fn().mockResolvedValue(true);
const listenMock = vi.fn();
const unlistenSpy = vi.fn();
const relaunchMock = vi.fn();

vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: (...args: unknown[]) => relaunchMock(...args),
}));

// `listen` is the download-progress channel. Most tests never drive
// an install, so the default (set in `beforeEach`) just resolves to
// the unlisten spy; the install test installs a capturing impl.
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

// The provider probes platform support and drives all update flow
// through the Tauri-bound `api`. Stub the whole surface; individual
// mocks are overridden per test.
vi.mock("../api", () => ({
  api: {
    updaterSupported: () => updaterSupportedMock(),
    releaseUpdateCheck: () => releaseUpdateCheckMock(),
    releaseUpdateInstall: () => releaseUpdateInstallMock(),
    releaseChannelGet: () => releaseChannelGetMock(),
    releaseChannelSet: (...args: unknown[]) => releaseChannelSetMock(...args),
    relaunchBusyOps: () => relaunchBusyOpsMock(),
  },
}));

const wrapper = ({ children }: { children: ReactNode }) => (
  <UpdateProvider>{children}</UpdateProvider>
);

/** A no-update check result — the common backdrop for scheduler tests. */
const UP_TO_DATE_DTO = {
  updateAvailable: false,
  version: null,
  currentVersion: "0.1.39",
  notes: null,
  pubDate: null,
  channel: "stable",
  strandedOnPrerelease: false,
  stableVersion: null,
};

/**
 * Reset every mock to its baseline. Shared by the per-describe
 * `beforeEach` hooks so the fake-timer describes don't drift from
 * the lifecycle describe's setup.
 */
function resetUpdateMocks() {
  releaseUpdateCheckMock.mockReset();
  releaseUpdateInstallMock.mockReset();
  releaseChannelGetMock.mockReset();
  releaseChannelSetMock.mockReset();
  relaunchBusyOpsMock.mockReset();
  updaterSupportedMock.mockReset();
  listenMock.mockReset();
  unlistenSpy.mockReset();
  relaunchMock.mockReset();
  listenMock.mockResolvedValue(unlistenSpy);
  updaterSupportedMock.mockResolvedValue(true);
  releaseChannelGetMock.mockResolvedValue("stable");
  releaseChannelSetMock.mockImplementation((channel: string) =>
    Promise.resolve(channel),
  );
  relaunchBusyOpsMock.mockResolvedValue([]);
  relaunchMock.mockResolvedValue(undefined);
  try {
    localStorage.clear();
  } catch {
    // jsdom storage may not be present in some envs.
  }
}

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
    resetUpdateMocks();
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

  it("loads the release channel from the Rust preference on mount", async () => {
    releaseChannelGetMock.mockResolvedValue("beta");
    const { result } = renderHook(() => useUpdater(), { wrapper });
    await waitFor(() => expect(result.current.releaseChannel).toBe("beta"));
  });

  it("transitions to 'up-to-date' when no update is returned", async () => {
    releaseUpdateCheckMock.mockResolvedValue({
      updateAvailable: false,
      version: null,
      currentVersion: "0.1.39",
      notes: null,
      pubDate: null,
      channel: "stable",
    });
    const { result } = renderHook(() => useUpdater(), { wrapper });

    await act(async () => {
      await result.current.checkNow();
    });
    expect(result.current.status).toBe("up-to-date");
    expect(result.current.updateInfo).toBeNull();
    expect(result.current.lastCheckedAt).not.toBeNull();
  });

  it("transitions to 'available' and exposes UpdateInfo when an update is returned", async () => {
    releaseUpdateCheckMock.mockResolvedValue({
      updateAvailable: true,
      version: "0.0.7",
      currentVersion: "0.0.6",
      notes: "release notes",
      pubDate: "2026-04-28",
      channel: "stable",
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
      pubDate: "2026-04-28",
    });
  });

  it("transitions to 'error' and surfaces the message when check throws", async () => {
    releaseUpdateCheckMock.mockRejectedValue(new Error("network down"));
    const { result } = renderHook(() => useUpdater(), { wrapper });

    await act(async () => {
      await result.current.checkNow();
    });
    expect(result.current.status).toBe("error");
    expect(result.current.error).toBe("network down");
  });

  it("skipThisVersion sets isSkipped, resetSkip clears it", async () => {
    releaseUpdateCheckMock.mockResolvedValue({
      updateAvailable: true,
      version: "0.0.7",
      currentVersion: "0.0.6",
      notes: "",
      pubDate: null,
      channel: "stable",
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

  it("setReleaseChannel optimistically updates and persists via the Rust command", async () => {
    const { result } = renderHook(() => useUpdater(), { wrapper });
    await waitFor(() => expect(result.current.releaseChannel).toBe("stable"));

    await act(async () => {
      result.current.setReleaseChannel("beta");
    });
    // Optimistic local update is immediate.
    expect(result.current.releaseChannel).toBe("beta");
    // And the Rust setter was invoked with the new channel.
    expect(releaseChannelSetMock).toHaveBeenCalledWith("beta");
  });

  it("setReleaseChannel keeps the optimistic value and surfaces an error when the Rust setter rejects", async () => {
    releaseChannelSetMock.mockRejectedValue(new Error("disk full"));
    const { result } = renderHook(() => useUpdater(), { wrapper });
    await waitFor(() => expect(result.current.releaseChannel).toBe("stable"));

    await act(async () => {
      result.current.setReleaseChannel("beta");
    });
    // The Rust persist failed, but the optimistic value is retained
    // (the next check simply uses whatever Rust actually has) and the
    // failure is surfaced.
    await waitFor(() =>
      expect(result.current.error).toContain("disk full"),
    );
    expect(result.current.releaseChannel).toBe("beta");
  });

  it("drives the download → ready path and maps started/progress/finished events", async () => {
    // Capture the progress callback `downloadAndInstall` registers.
    let progressCb: ((ev: { payload: unknown }) => void) | null = null;
    listenMock.mockImplementation(
      (_event: string, cb: (ev: { payload: unknown }) => void) => {
        progressCb = cb;
        return Promise.resolve(unlistenSpy);
      },
    );
    releaseUpdateCheckMock.mockResolvedValue({
      updateAvailable: true,
      version: "0.0.7",
      currentVersion: "0.0.6",
      notes: "",
      pubDate: null,
      channel: "stable",
    });
    // Rust emits the three progress frames during the install call,
    // then resolves.
    releaseUpdateInstallMock.mockImplementation(() => {
      progressCb?.({ payload: { event: "started", contentLength: 1000 } });
      progressCb?.({
        payload: { event: "progress", downloaded: 400, contentLength: 1000 },
      });
      progressCb?.({ payload: { event: "finished" } });
      return Promise.resolve();
    });

    const { result } = renderHook(() => useUpdater(), { wrapper });
    await act(async () => {
      await result.current.checkNow();
    });
    expect(result.current.status).toBe("available");

    await act(async () => {
      await result.current.downloadAndInstall();
    });
    expect(result.current.status).toBe("ready");
    // `finished` settles `downloaded` to the known total.
    expect(result.current.downloadProgress).toEqual({
      downloaded: 1000,
      total: 1000,
    });
    // The progress listener must be torn down after the install.
    expect(unlistenSpy).toHaveBeenCalled();
  });

  it("transitions to 'error' and unsubscribes when the install rejects", async () => {
    listenMock.mockResolvedValue(unlistenSpy);
    releaseUpdateCheckMock.mockResolvedValue({
      updateAvailable: true,
      version: "0.0.7",
      currentVersion: "0.0.6",
      notes: "",
      pubDate: null,
      channel: "stable",
    });
    releaseUpdateInstallMock.mockRejectedValue(new Error("install failed"));

    const { result } = renderHook(() => useUpdater(), { wrapper });
    await act(async () => {
      await result.current.checkNow();
    });
    await act(async () => {
      await result.current.downloadAndInstall();
    });
    expect(result.current.status).toBe("error");
    expect(result.current.error).toBe("install failed");
    // The listener is torn down on the failure path too (`finally`).
    expect(unlistenSpy).toHaveBeenCalled();
  });

  it("setReleaseChannel resets a prior check result back to idle", async () => {
    releaseUpdateCheckMock.mockResolvedValue({
      updateAvailable: true,
      version: "0.2.0-beta.1",
      currentVersion: "0.1.46",
      notes: "",
      pubDate: null,
      channel: "beta",
      strandedOnPrerelease: false,
      stableVersion: null,
    });
    const { result } = renderHook(() => useUpdater(), { wrapper });
    await act(async () => {
      await result.current.checkNow();
    });
    expect(result.current.status).toBe("available");
    expect(result.current.updateInfo).not.toBeNull();

    // Switching channels must not leave the *other* channel's
    // "Update available" card (and its stale Rust-side handle)
    // standing — the renderer resets until a fresh check runs.
    await act(async () => {
      result.current.setReleaseChannel("stable");
    });
    expect(result.current.status).toBe("idle");
    expect(result.current.updateInfo).toBeNull();
    expect(result.current.downloadProgress).toBeNull();
  });

  it("exposes stranded info when the check reports stranded-on-prerelease", async () => {
    releaseUpdateCheckMock.mockResolvedValue({
      updateAvailable: false,
      version: null,
      currentVersion: "0.2.0-beta.1",
      notes: null,
      pubDate: null,
      channel: "stable",
      strandedOnPrerelease: true,
      stableVersion: "0.1.46",
    });
    const { result } = renderHook(() => useUpdater(), { wrapper });
    await act(async () => {
      await result.current.checkNow();
    });
    // Status is up-to-date at the machine level, but the stranded
    // marker lets the badge avoid the false "latest version" claim.
    expect(result.current.status).toBe("up-to-date");
    expect(result.current.stranded).toEqual({ stableVersion: "0.1.46" });

    // A later non-stranded check clears the marker.
    releaseUpdateCheckMock.mockResolvedValue(UP_TO_DATE_DTO);
    await act(async () => {
      await result.current.checkNow();
    });
    expect(result.current.stranded).toBeNull();
  });

  it("applyUpdate relaunches directly when no background ops are running", async () => {
    relaunchBusyOpsMock.mockResolvedValue([]);
    const { result } = renderHook(() => useUpdater(), { wrapper });
    await act(async () => {
      await result.current.applyUpdate();
    });
    expect(relaunchMock).toHaveBeenCalledTimes(1);
  });

  it("applyUpdate warn-confirms instead of relaunching while ops are in flight", async () => {
    relaunchBusyOpsMock.mockResolvedValue(["Renaming proj-a → proj-b"]);
    const { result } = renderHook(() => useUpdater(), { wrapper });
    await act(async () => {
      await result.current.applyUpdate();
    });
    // No relaunch yet — the provider renders the confirm dialog.
    expect(relaunchMock).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByText("Renaming proj-a → proj-b")).toBeInTheDocument();

    // Cancel keeps the app alive and closes the dialog.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    });
    expect(relaunchMock).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();

    // Re-trigger and confirm — only then does the relaunch fire.
    await act(async () => {
      await result.current.applyUpdate();
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Restart anyway" }));
    });
    expect(relaunchMock).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("applyUpdate still relaunches when the busy probe itself fails", async () => {
    // A broken probe must not strand the user behind a dead
    // "Restart to update" button.
    relaunchBusyOpsMock.mockRejectedValue(new Error("ipc down"));
    const { result } = renderHook(() => useUpdater(), { wrapper });
    await act(async () => {
      await result.current.applyUpdate();
    });
    expect(relaunchMock).toHaveBeenCalledTimes(1);
  });
});

/**
 * Timing-sensitive paths: the startup auto-check (2 s hydration
 * delay, fires at most once per launch) and scheduleRetry's
 * 5 s → 10 s → 20 s exponential backoff with RETRY_MAX give-up.
 * Everything here runs under fake timers; `advanceTimersByTimeAsync`
 * also flushes the microtasks the mocked api promises resolve on.
 */
describe("UpdateProvider — startup auto-check and retry backoff", () => {
  beforeEach(() => {
    resetUpdateMocks();
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  /** Mount the hook and let the support probe + channel load settle. */
  async function mountSettled() {
    const rendered = renderHook(() => useUpdater(), { wrapper });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(rendered.result.current.supported).toBe(true);
    return rendered;
  }

  it("fires the startup auto-check exactly once, after the 2 s delay", async () => {
    releaseUpdateCheckMock.mockResolvedValue(UP_TO_DATE_DTO);
    const { result } = await mountSettled();
    expect(releaseUpdateCheckMock).not.toHaveBeenCalled();

    // Not a millisecond early…
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_999);
    });
    expect(releaseUpdateCheckMock).not.toHaveBeenCalled();

    // …fires at exactly STARTUP_DELAY_MS…
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(releaseUpdateCheckMock).toHaveBeenCalledTimes(1);
    expect(result.current.status).toBe("up-to-date");

    // …and never again this launch.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60 * 60_000);
    });
    expect(releaseUpdateCheckMock).toHaveBeenCalledTimes(1);
  });

  it("does not auto-check at all when frequency is 'manual'", async () => {
    localStorage.setItem("claudepot.update.checkFrequency", "manual");
    releaseUpdateCheckMock.mockResolvedValue(UP_TO_DATE_DTO);
    await mountSettled();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(60 * 60_000);
    });
    expect(releaseUpdateCheckMock).not.toHaveBeenCalled();
  });

  it("retries a failed background check at 5 s / 10 s / 20 s, then gives up", async () => {
    releaseUpdateCheckMock.mockRejectedValue(new Error("offline"));
    const { result } = await mountSettled();

    // Startup check fails → first retry armed at +5 s.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    expect(releaseUpdateCheckMock).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(4_999);
    });
    expect(releaseUpdateCheckMock).toHaveBeenCalledTimes(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(releaseUpdateCheckMock).toHaveBeenCalledTimes(2);

    // Second retry at +10 s, third at +20 s.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(releaseUpdateCheckMock).toHaveBeenCalledTimes(3);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20_000);
    });
    expect(releaseUpdateCheckMock).toHaveBeenCalledTimes(4);

    // RETRY_MAX reached — the backoff never hammers GitHub again.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(24 * 60 * 60_000);
    });
    expect(releaseUpdateCheckMock).toHaveBeenCalledTimes(4);
    expect(result.current.status).toBe("error");
  });

  it("a successful check cancels the pending retry and resets the backoff", async () => {
    releaseUpdateCheckMock.mockRejectedValueOnce(new Error("offline"));
    releaseUpdateCheckMock.mockResolvedValue(UP_TO_DATE_DTO);
    const { result } = await mountSettled();

    // Startup check fails → retry pending at +5 s.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    expect(releaseUpdateCheckMock).toHaveBeenCalledTimes(1);

    // The retry fires and succeeds — the success path cancels the
    // backoff state, so nothing further is ever queued.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(releaseUpdateCheckMock).toHaveBeenCalledTimes(2);
    expect(result.current.status).toBe("up-to-date");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(24 * 60 * 60_000);
    });
    expect(releaseUpdateCheckMock).toHaveBeenCalledTimes(2);
  });

  it("a manual checkNow cancels the pending retry and does not re-arm it", async () => {
    releaseUpdateCheckMock.mockRejectedValue(new Error("offline"));
    const { result } = await mountSettled();

    // Startup check fails → retry pending at +5 s.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    expect(releaseUpdateCheckMock).toHaveBeenCalledTimes(1);

    // Manual check 1 s later supersedes the queued retry. It also
    // fails — but manual failures never auto-retry.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    await act(async () => {
      await result.current.checkNow();
    });
    expect(releaseUpdateCheckMock).toHaveBeenCalledTimes(2);

    // Neither the original +5 s slot nor any backoff window fires.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(24 * 60 * 60_000);
    });
    expect(releaseUpdateCheckMock).toHaveBeenCalledTimes(2);
    expect(result.current.status).toBe("error");
  });
});
