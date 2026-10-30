import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";

import type { RelayLive, RelayStatus } from "../api/relay";

// The relay's three tiers, each with one job: the log row for a
// non-blocking upstream error (useRelayErrorNotifications), the banner
// for a blocking condition (useRelayIssues). A refactor that let one
// event reach both would land green without these.

const emitMock = vi.fn();
vi.mock("../providers/AppStateProvider", () => ({
  useEmit: () => emitMock,
  useAppState: () => ({}),
}));

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

const settingsTabMock = vi.fn();
vi.mock("../lib/networkPanelDeepLink", () => ({
  triggerSettingsTab: (tab: string) => settingsTabMock(tab),
}));

import { useRelayErrorNotifications } from "./useRelayErrorNotifications";
import { useRelayIssues } from "./useRelayIssues";

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

async function fire(payload: RelayLive) {
  await waitFor(() => expect(listeners.has("relay-status")).toBe(true));
  act(() => listeners.get("relay-status")!({ payload }));
}

beforeEach(() => {
  listeners.clear();
  emitMock.mockReset().mockResolvedValue(undefined);
  relayStatusMock.mockReset().mockResolvedValue(status());
  settingsTabMock.mockReset();
});

/** A serving payload whose upstreams carry these errors, with the
 *  aggregate fields derived as `RelayStatusEvent::from_live` does. The
 *  codes are ones core emits as a `credential_error`
 *  (`Refusal::Upstream(..).code()`); `relay_server`'s
 *  `an_upstream_credential_error_reaches_the_payload_as_its_code` pins
 *  that end. */
function errs(byId: Record<string, string | null>): RelayLive {
  const upstreams = Object.entries(byId).map(([id, error]) => ({ id, ready: error === null, error }));
  const codes = upstreams.map((u) => u.error).filter((e): e is string => e !== null);
  const blocking = (codes.find((c) => c === "owner_conflict") ??
    codes.find((c) => c === "identity_changed") ??
    null) as RelayLive["blocking"];
  const lastError = codes.find((c) => c !== "owner_conflict" && c !== "identity_changed") ?? null;
  return live({ upstreams, blocking, lastError });
}

describe("useRelayErrorNotifications", () => {
  it("logs a new upstream error once, under relayError", async () => {
    renderHook(() => useRelayErrorNotifications());
    await fire(errs({ a: "credential_unavailable" }));
    await fire(errs({ a: "credential_unavailable" }));
    expect(emitMock).toHaveBeenCalledTimes(1);
    expect(emitMock.mock.calls[0][0]).toMatchObject({
      category: "relayError",
      dedupeKey: "relay:a:credential_unavailable",
    });
  });

  it("logs the same error again after a clean tick", async () => {
    renderHook(() => useRelayErrorNotifications());
    await fire(errs({ a: "credential_unavailable" }));
    await fire(errs({ a: null }));
    await fire(errs({ a: "credential_unavailable" }));
    expect(emitMock).toHaveBeenCalledTimes(2);
  });

  it("leaves a blocking condition to the banner", async () => {
    renderHook(() => useRelayErrorNotifications());
    await fire(errs({ a: "owner_conflict" }));
    expect(emitMock).not.toHaveBeenCalled();
  });

  it("logs a second upstream's failure while the first is still failing", async () => {
    renderHook(() => useRelayErrorNotifications());
    await fire(errs({ a: "credential_unavailable", b: null }));
    await fire(errs({ a: "credential_unavailable", b: "upstream_unreachable" }));
    expect(emitMock.mock.calls.map((c) => c[0].dedupeKey)).toEqual([
      "relay:a:credential_unavailable",
      "relay:b:upstream_unreachable",
    ]);
  });

  it("logs the same code once per upstream that hits it", async () => {
    renderHook(() => useRelayErrorNotifications());
    await fire(errs({ a: "credential_unavailable", b: "credential_unavailable" }));
    expect(emitMock).toHaveBeenCalledTimes(2);
  });

  it("tracks recovery per upstream: one recovering does not re-log the other", async () => {
    renderHook(() => useRelayErrorNotifications());
    await fire(errs({ a: "credential_unavailable", b: "upstream_unreachable" }));
    await fire(errs({ a: null, b: "upstream_unreachable" }));
    await fire(errs({ a: "credential_unavailable", b: "upstream_unreachable" }));
    expect(emitMock.mock.calls.map((c) => c[0].dedupeKey)).toEqual([
      "relay:a:credential_unavailable",
      "relay:b:upstream_unreachable",
      "relay:a:credential_unavailable",
    ]);
  });

  it("logs a non-blocking failure beside a blocked upstream", async () => {
    renderHook(() => useRelayErrorNotifications());
    await fire(errs({ a: "owner_conflict", b: "credential_unavailable" }));
    expect(emitMock.mock.calls.map((c) => c[0].dedupeKey)).toEqual(["relay:b:credential_unavailable"]);
  });

  it("forgets every upstream when the relay stops", async () => {
    renderHook(() => useRelayErrorNotifications());
    await fire(errs({ a: "credential_unavailable" }));
    await fire(live({ serving: false }));
    await fire(errs({ a: "credential_unavailable" }));
    expect(emitMock).toHaveBeenCalledTimes(2);
  });
});

describe("useRelayIssues", () => {
  it("raises an undismissable error banner for a blocking condition", async () => {
    const { result } = renderHook(() => useRelayIssues(() => {}));
    await fire(live({ blocking: "identity_changed", lastError: "identity_changed" }));
    await waitFor(() => expect(result.current).toHaveLength(1));
    expect(result.current[0]).toMatchObject({ id: "relay:identity_changed", severity: "error" });
    expect(result.current[0].dismissable).toBeFalsy();
  });

  it("raises one for an unreadable CA key and deep-links to the Relay pane", async () => {
    relayStatusMock.mockResolvedValue(status({ lastErrorCode: "relay_ca_key_unavailable" }));
    const setSection = vi.fn();
    const { result } = renderHook(() => useRelayIssues(setSection));
    await waitFor(() => expect(result.current).toHaveLength(1));
    expect(result.current[0].id).toBe("relay:ca_key_unavailable");
    result.current[0].action!.onClick();
    expect(settingsTabMock).toHaveBeenCalledWith("relay");
    expect(setSection).toHaveBeenCalledWith("settings");
  });

  it("raises nothing for a non-blocking upstream error", async () => {
    const { result } = renderHook(() => useRelayIssues(() => {}));
    await fire(live({ lastError: "credential_unavailable" }));
    expect(result.current).toEqual([]);
  });
});
