import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";

// Mock the api module before importing the hook so the hook's import
// of `api` resolves to our stub. The api index pulls `invoke` from
// the Tauri bridge which doesn't exist under jsdom; mocking at this
// level keeps the hook test pure-renderer.
const networkFirstRunCheck = vi.fn();
vi.mock("../api", () => ({
  api: {
    networkFirstRunCheck: () => networkFirstRunCheck(),
  },
}));

import { useNetworkGate } from "./useNetworkGate";

beforeEach(() => {
  networkFirstRunCheck.mockReset();
  sessionStorage.clear();
});

describe("useNetworkGate", () => {
  it("starts in 'unknown' synchronously, before the probe resolves", () => {
    // Use a probe that never resolves so we can observe the pre-probe
    // state without racing the resolution.
    networkFirstRunCheck.mockReturnValue(new Promise(() => {}));
    const { result } = renderHook(() => useNetworkGate({ probeDelayMs: 0 }));
    expect(result.current.state.kind).toBe("unknown");
    expect(result.current.shouldShowPanel).toBe(false);
  });

  it("transitions to 'ok' when the probe reports reachable", async () => {
    networkFirstRunCheck.mockResolvedValue({
      diagnosis: "reachable",
      latencyMs: 87,
      message: null,
    });
    const { result } = renderHook(() => useNetworkGate({ probeDelayMs: 0 }));
    await waitFor(() => {
      expect(result.current.state.kind).toBe("ok");
    });
    if (result.current.state.kind === "ok") {
      expect(result.current.state.latencyMs).toBe(87);
    }
    expect(result.current.shouldShowPanel).toBe(false);
  });

  it("transitions to 'unreachable' and surfaces the panel on dns_failure", async () => {
    networkFirstRunCheck.mockResolvedValue({
      diagnosis: "dns_failure",
      latencyMs: null,
      message: "dns lookup failed",
    });
    const { result } = renderHook(() => useNetworkGate({ probeDelayMs: 0 }));
    await waitFor(() => {
      expect(result.current.state.kind).toBe("unreachable");
    });
    expect(result.current.shouldShowPanel).toBe(true);
    if (result.current.state.kind === "unreachable") {
      expect(result.current.state.diagnosis).toBe("dns_failure");
      expect(result.current.state.message).toBe("dns lookup failed");
    }
  });

  it("dismiss() hides the panel and persists across remount", async () => {
    networkFirstRunCheck.mockResolvedValue({
      diagnosis: "timeout",
      latencyMs: null,
      message: "connection timed out",
    });
    const { result, unmount } = renderHook(() =>
      useNetworkGate({ probeDelayMs: 0 }),
    );
    await waitFor(() => {
      expect(result.current.shouldShowPanel).toBe(true);
    });

    act(() => {
      result.current.dismiss();
    });
    expect(result.current.shouldShowPanel).toBe(false);

    // Remount — sessionStorage flag carries the dismissal forward.
    unmount();
    const { result: result2 } = renderHook(() =>
      useNetworkGate({ probeDelayMs: 0 }),
    );
    await waitFor(() => {
      // State still goes to 'unreachable' (the network is still
      // broken), but the panel stays hidden.
      expect(result2.current.state.kind).toBe("unreachable");
    });
    expect(result2.current.shouldShowPanel).toBe(false);
  });

  it("retry() re-runs the probe and clears unreachable on recovery", async () => {
    // First probe fails, second succeeds — the panel should disappear
    // after retry().
    networkFirstRunCheck
      .mockResolvedValueOnce({
        diagnosis: "connection_refused",
        latencyMs: null,
        message: "refused",
      })
      .mockResolvedValueOnce({
        diagnosis: "reachable",
        latencyMs: 42,
        message: null,
      });

    const { result } = renderHook(() => useNetworkGate({ probeDelayMs: 0 }));
    await waitFor(() => {
      expect(result.current.state.kind).toBe("unreachable");
    });
    expect(result.current.shouldShowPanel).toBe(true);

    act(() => {
      result.current.retry();
    });
    await waitFor(() => {
      expect(result.current.state.kind).toBe("ok");
    });
    expect(result.current.shouldShowPanel).toBe(false);
  });

  it("retry()'s newer result wins when a slow original probe resolves later", async () => {
    // Audit-flagged race: original probe is slow + fails; user hits
    // Retry; retry probe is fast + succeeds; THEN the original
    // resolves with its old failure. Without a generation guard the
    // original would clobber the newer 'ok' state. Verify it doesn't.
    let resolveFirst: ((v: unknown) => void) | null = null;
    let resolveSecond: ((v: unknown) => void) | null = null;
    networkFirstRunCheck
      .mockReturnValueOnce(
        new Promise((res) => {
          resolveFirst = res;
        }),
      )
      .mockReturnValueOnce(
        new Promise((res) => {
          resolveSecond = res;
        }),
      );

    const { result } = renderHook(() => useNetworkGate({ probeDelayMs: 0 }));
    // Wait for the hook to start the first probe — state goes to
    // "probing" once setTimeout fires.
    await waitFor(() => {
      expect(result.current.state.kind).toBe("probing");
    });

    // User clicks Retry while the first probe is still in flight.
    act(() => {
      result.current.retry();
    });

    // Resolve the SECOND probe first — newer generation wins.
    await act(async () => {
      resolveSecond?.({
        diagnosis: "reachable",
        latencyMs: 30,
        message: null,
      });
    });
    await waitFor(() => {
      expect(result.current.state.kind).toBe("ok");
    });

    // Now the original probe resolves with a stale "unreachable"
    // result. The hook's generation guard must drop it.
    await act(async () => {
      resolveFirst?.({
        diagnosis: "dns_failure",
        latencyMs: null,
        message: "stale",
      });
      // Yield once more so any errant setState propagates.
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(result.current.state.kind).toBe("ok");
    expect(result.current.shouldShowPanel).toBe(false);
  });

  it("does not setState after unmount when an in-flight probe resolves late", async () => {
    // React will warn about state updates on unmounted components.
    // Spy on console.error — that's where React's warning lands —
    // and assert nothing was logged about the unmounted hook.
    const consoleError = vi
      .spyOn(console, "error")
      .mockImplementation(() => {});

    let resolveProbe: ((v: unknown) => void) | null = null;
    networkFirstRunCheck.mockReturnValueOnce(
      new Promise((res) => {
        resolveProbe = res;
      }),
    );

    const { unmount, result } = renderHook(() =>
      useNetworkGate({ probeDelayMs: 0 }),
    );
    await waitFor(() => {
      expect(result.current.state.kind).toBe("probing");
    });

    // Unmount BEFORE the probe resolves.
    unmount();

    // Now resolve the probe. The hook must not call setState.
    await act(async () => {
      resolveProbe?.({
        diagnosis: "reachable",
        latencyMs: 10,
        message: null,
      });
      await new Promise((r) => setTimeout(r, 0));
    });

    // No React-warning about updating state on unmounted component.
    const calls = consoleError.mock.calls.map((args) => args.join(" "));
    expect(
      calls.some((c) => /unmounted|memory leak/i.test(c)),
    ).toBe(false);
    consoleError.mockRestore();
  });

  it("treats a probe-command failure as 'unknown', not 'unreachable'", async () => {
    // A Tauri-bridge fault should not present as "Anthropic
    // unreachable" — that would mislead the user about the actual
    // failure mode.
    const consoleWarn = vi.spyOn(console, "warn").mockImplementation(() => {});
    networkFirstRunCheck.mockRejectedValue(new Error("ipc bridge gone"));
    const { result } = renderHook(() => useNetworkGate({ probeDelayMs: 0 }));
    // Wait for the catch handler to run by giving the rejected
    // promise a tick to settle. Since state goes back to "unknown"
    // (the same as initial), waitFor on state.kind would resolve
    // immediately on the initial state. Use the warn-spy as the
    // signal that the catch fired.
    await waitFor(() => {
      expect(consoleWarn).toHaveBeenCalled();
    });
    expect(result.current.state.kind).toBe("unknown");
    expect(result.current.shouldShowPanel).toBe(false);
    consoleWarn.mockRestore();
  });
});
