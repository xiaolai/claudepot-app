import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, renderHook, screen } from "@testing-library/react";

import type { RelayLive, RelayStatus } from "../api/relay";
import { i18n } from "../lib/i18n";

// The ambient read behind the status-bar chip and the relay banners.
// Each ordering rule below was a way for the chip to show a state the
// relay is not in.

type Listener = (ev: { payload?: unknown }) => void;
const listeners = new Map<string, Listener>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (channel: string, fn: Listener) => {
    listeners.set(channel, fn);
    return () => listeners.delete(channel);
  },
}));

const relayStatusMock = vi.fn();
vi.mock("../api", () => ({
  relayApi: { relayStatus: (...a: unknown[]) => relayStatusMock(...a) },
}));

vi.mock("../lib/networkPanelDeepLink", () => ({ triggerSettingsTab: () => {} }));

import { useRelayStatus } from "./useRelayStatus";
import { useRelayIssues } from "./useRelayIssues";
import { RelayServingChip } from "../components/RelayServingChip";

function live(over: Partial<RelayLive> = {}): RelayLive {
  return { serving: true, activeStreams: 0, blocking: null, lastError: null, upstreams: [], ...over };
}

function status(over: Partial<RelayStatus> = {}): RelayStatus {
  return {
    serving: false,
    runningHere: false,
    addr: null,
    bind: "127.0.0.1",
    port: 8787,
    exposure: "loopback",
    bindError: null,
    caFingerprint: null,
    configRecovered: false,
    devicesRecovered: false,
    usageUnavailable: false,
    publications: [],
    devices: [],
    accounts: [],
    lastError: null,
    lastErrorCode: null,
    warnings: [],
    liveRevokeUnavailable: null,
    revokeSocketPresent: false,
    live: live({ serving: false }),
    ...over,
  };
}

const SERVING = { serving: true, live: live() } as const;

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

const flush = (ms = 0) =>
  act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });

const tc = (k: string, o?: Record<string, unknown>) =>
  i18n.t(k as never, { ns: "components", ...o }) as string;

beforeEach(() => {
  vi.useFakeTimers();
  listeners.clear();
  relayStatusMock.mockReset().mockResolvedValue(status());
});
afterEach(() => vi.useRealTimers());

describe("useRelayStatus", () => {
  it("reads on mount and again every ten seconds", async () => {
    renderHook(() => useRelayStatus());
    await flush();
    expect(relayStatusMock).toHaveBeenCalledTimes(1);
    await flush(9_999);
    expect(relayStatusMock).toHaveBeenCalledTimes(1);
    await flush(1);
    expect(relayStatusMock).toHaveBeenCalledTimes(2);
  });

  it("picks up a relay started from a terminal on the next poll", async () => {
    const { result } = renderHook(() => useRelayStatus());
    await flush();
    expect(result.current?.serving).toBe(false);
    relayStatusMock.mockResolvedValue(status(SERVING));
    await flush(10_000);
    expect(result.current?.serving).toBe(true);
  });

  it("keeps the last known state, marked stale, when a read fails", async () => {
    relayStatusMock.mockResolvedValueOnce(status(SERVING)).mockRejectedValue(new Error("account store locked"));
    const { result } = renderHook(() => useRelayStatus());
    await flush();
    expect(result.current).toMatchObject({ serving: true, stale: false });
    await flush(10_000);
    expect(result.current).toMatchObject({ serving: true, stale: true });
  });

  it("clears the stale mark on the next good read", async () => {
    relayStatusMock
      .mockResolvedValueOnce(status(SERVING))
      .mockRejectedValueOnce(new Error("transient"))
      .mockResolvedValue(status(SERVING));
    const { result } = renderHook(() => useRelayStatus());
    await flush();
    await flush(10_000);
    expect(result.current?.stale).toBe(true);
    await flush(10_000);
    expect(result.current?.stale).toBe(false);
  });

  it("stays null when the very first read fails", async () => {
    relayStatusMock.mockRejectedValue(new Error("no backend"));
    const { result } = renderHook(() => useRelayStatus());
    await flush();
    expect(result.current).toBeNull();
  });

  it("lands a read slower than the poll interval instead of re-reading over it", async () => {
    // Each read takes 12 s, longer than the 10 s poll.
    relayStatusMock.mockImplementation(
      () => new Promise<RelayStatus>((res) => setTimeout(() => res(status(SERVING)), 12_000)),
    );
    const { result } = renderHook(() => useRelayStatus());
    await flush(12_000);
    expect(result.current?.serving).toBe(true);
    // The poll that fell due at 10 s was skipped, not queued behind it.
    expect(relayStatusMock).toHaveBeenCalledTimes(1);
    await flush(8_000); // the 20 s poll
    expect(relayStatusMock).toHaveBeenCalledTimes(2);
  });

  it("lets an event outrank a read still in flight", async () => {
    const inflight = deferred<RelayStatus>();
    relayStatusMock.mockReturnValueOnce(inflight.promise);
    const { result } = renderHook(() => useRelayStatus());
    await flush();
    act(() => listeners.get("relay-status")!({ payload: live({ activeStreams: 2 }) }));
    expect(result.current).toMatchObject({ serving: true, live: { activeStreams: 2 } });
    inflight.resolve(status());
    await flush();
    expect(result.current?.serving).toBe(true);
  });

  it("keeps a read's CA fault when an event lands while the read is in flight", async () => {
    const inflight = deferred<RelayStatus>();
    relayStatusMock.mockReturnValueOnce(inflight.promise);
    const { result } = renderHook(() => useRelayStatus());
    await flush();
    act(() => listeners.get("relay-status")!({ payload: live({ activeStreams: 1 }) }));
    inflight.resolve(status({ lastErrorCode: "relay_ca_key_unavailable" }));
    await flush();
    // Live fields stay the event's; the poll-only field is the read's.
    expect(result.current).toMatchObject({
      serving: true,
      live: { activeStreams: 1 },
      lastErrorCode: "relay_ca_key_unavailable",
    });
  });

  it("raises the CA banner while events keep streaming over every read", async () => {
    const reads = [deferred<RelayStatus>(), deferred<RelayStatus>()];
    relayStatusMock.mockReturnValueOnce(reads[0].promise).mockReturnValueOnce(reads[1].promise);
    const { result } = renderHook(() => useRelayIssues(() => {}));
    await flush();
    for (const [i, r] of reads.entries()) {
      if (i > 0) await flush(10_000); // the next poll
      act(() => listeners.get("relay-status")!({ payload: live({ activeStreams: 1 }) }));
      r.resolve(status({ serving: true, lastErrorCode: "relay_ca_key_unavailable" }));
      await flush();
    }
    expect(relayStatusMock).toHaveBeenCalledTimes(2);
    expect(result.current.map((i) => i.id)).toEqual(["relay:ca_key_unavailable"]);
  });

  it("stops polling and drops a late reply after unmount", async () => {
    const late = deferred<RelayStatus>();
    relayStatusMock.mockReturnValueOnce(late.promise);
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    const { result, unmount } = renderHook(() => useRelayStatus());
    await flush();
    unmount();
    late.resolve(status(SERVING));
    await flush(60_000);
    expect(relayStatusMock).toHaveBeenCalledTimes(1);
    expect(result.current).toBeNull();
    expect(errors).not.toHaveBeenCalled();
    errors.mockRestore();
  });
});

describe("RelayServingChip", () => {
  it("renders nothing while nothing serves", async () => {
    const { container } = render(<RelayServingChip />);
    await flush();
    expect(container).toBeEmptyDOMElement();
  });

  it("says the relay is on, without a zero stream count", async () => {
    relayStatusMock.mockResolvedValue(status(SERVING));
    render(<RelayServingChip />);
    await flush();
    const chip = screen.getByRole("status");
    expect(chip).toHaveTextContent(tc("relayServing.serving"));
    expect(chip).toHaveAttribute("title", tc("relayServing.hint"));
    expect(chip.className).not.toContain("warn");
  });

  it("counts streams and warns while an upstream is blocked", async () => {
    relayStatusMock.mockResolvedValue(
      status({ serving: true, live: live({ activeStreams: 3, blocking: "owner_conflict" }) }),
    );
    render(<RelayServingChip />);
    await flush();
    const chip = screen.getByRole("status");
    expect(chip).toHaveTextContent("relay · 3 streams");
    expect(chip.className).toContain("warn");
  });

  it("stays lit on a failed read and says the state is not current", async () => {
    relayStatusMock.mockResolvedValueOnce(status(SERVING)).mockRejectedValue(new Error("boom"));
    render(<RelayServingChip />);
    await flush();
    await flush(10_000);
    const chip = screen.getByRole("status");
    expect(chip).toHaveTextContent(tc("relayServing.serving"));
    expect(chip).toHaveAttribute("title", tc("relayServing.hintStale"));
  });
});

describe("useRelayIssues", () => {
  it("keeps a blocking banner through a failed read", async () => {
    relayStatusMock
      .mockResolvedValueOnce(status({ serving: true, live: live({ blocking: "owner_conflict" }) }))
      .mockRejectedValue(new Error("boom"));
    const { result } = renderHook(() => useRelayIssues(() => {}));
    await flush();
    expect(result.current).toHaveLength(1);
    await flush(10_000);
    expect(result.current.map((i) => i.id)).toEqual(["relay:owner_conflict"]);
  });

  it("re-renders an open banner in the new language after a switch", async () => {
    relayStatusMock.mockResolvedValue(status({ serving: true, live: live({ blocking: "owner_conflict" }) }));
    const { result } = renderHook(() => useRelayIssues(() => {}));
    await flush();
    const en = i18n.t("status.relay.owner_conflict.label", { lng: "en" });
    const zh = i18n.t("status.relay.owner_conflict.label", { lng: "zh-CN" });
    expect(zh).not.toBe(en);
    expect(result.current[0].label).toBe(en);
    try {
      await act(async () => {
        await i18n.changeLanguage("zh-CN");
      });
      expect(result.current[0].label).toBe(zh);
      expect(result.current[0].action!.label).toBe(i18n.t("status.relay.action", { lng: "zh-CN" }));
    } finally {
      await act(async () => {
        await i18n.changeLanguage("en");
      });
    }
  });
});
