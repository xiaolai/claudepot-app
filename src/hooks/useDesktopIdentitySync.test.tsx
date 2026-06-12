import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";

const syncFromCurrentDesktop = vi.fn();
vi.mock("../api", () => ({
  api: {
    syncFromCurrentDesktop: (...a: unknown[]) => syncFromCurrentDesktop(...a),
  },
}));

import { useDesktopIdentitySync } from "./useDesktopIdentitySync";

beforeEach(() => {
  syncFromCurrentDesktop.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("useDesktopIdentitySync", () => {
  it("probes unthrottled on mount and returns the outcome", async () => {
    syncFromCurrentDesktop.mockResolvedValue({ kind: "candidate_only" });
    const refreshAccounts = vi.fn().mockResolvedValue(undefined);
    const { result } = renderHook(() =>
      useDesktopIdentitySync(refreshAccounts),
    );
    expect(syncFromCurrentDesktop).toHaveBeenCalledTimes(1);
    await waitFor(() =>
      expect(result.current).toEqual({ kind: "candidate_only" }),
    );
    // candidate_only is not "verified" — no account refresh.
    expect(refreshAccounts).not.toHaveBeenCalled();
  });

  it("refreshes accounts when the sync verified a binding change", async () => {
    syncFromCurrentDesktop.mockResolvedValue({ kind: "verified" });
    const refreshAccounts = vi.fn().mockResolvedValue(undefined);
    renderHook(() => useDesktopIdentitySync(refreshAccounts));
    await waitFor(() => expect(refreshAccounts).toHaveBeenCalledTimes(1));
  });

  it("throttles focus-driven probes inside the 5-minute TTL", async () => {
    vi.useFakeTimers();
    syncFromCurrentDesktop.mockResolvedValue({ kind: "candidate_only" });
    const refreshAccounts = vi.fn().mockResolvedValue(undefined);
    renderHook(() => useDesktopIdentitySync(refreshAccounts));
    expect(syncFromCurrentDesktop).toHaveBeenCalledTimes(1);

    // Focus inside the TTL — throttled.
    act(() => {
      vi.advanceTimersByTime(60_000);
      window.dispatchEvent(new Event("focus"));
    });
    expect(syncFromCurrentDesktop).toHaveBeenCalledTimes(1);

    // Focus past the TTL — probes again.
    act(() => {
      vi.advanceTimersByTime(5 * 60_000);
      window.dispatchEvent(new Event("focus"));
    });
    expect(syncFromCurrentDesktop).toHaveBeenCalledTimes(2);
  });

  it("swallows probe failures (banner layer owns the messaging)", async () => {
    syncFromCurrentDesktop.mockRejectedValue(new Error("keychain locked"));
    const refreshAccounts = vi.fn();
    const { result } = renderHook(() =>
      useDesktopIdentitySync(refreshAccounts),
    );
    // Settle the rejected promise.
    await act(async () => {});
    expect(result.current).toBeNull();
    expect(refreshAccounts).not.toHaveBeenCalled();
  });
});
