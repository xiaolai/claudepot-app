import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { RelayDevice, RelayPublication, RelayStatus } from "../../api/relay";
import { i18n } from "../../lib/i18n";

const relayStatusMock = vi.fn();
const relayStartMock = vi.fn();
const relayStopMock = vi.fn();
const relaySetBindMock = vi.fn();
const relayPublishMock = vi.fn();
const relayUnpublishMock = vi.fn();
const relayRevokeDeviceMock = vi.fn();
const relayPairCodeMock = vi.fn();

vi.mock("../../api", () => ({
  relayApi: {
    relayStatus: (...a: unknown[]) => relayStatusMock(...a),
    relayStart: (...a: unknown[]) => relayStartMock(...a),
    relayStop: (...a: unknown[]) => relayStopMock(...a),
    relaySetBind: (...a: unknown[]) => relaySetBindMock(...a),
    relayPublish: (...a: unknown[]) => relayPublishMock(...a),
    relayUnpublish: (...a: unknown[]) => relayUnpublishMock(...a),
    relayRevokeDevice: (...a: unknown[]) => relayRevokeDeviceMock(...a),
    relayPairCode: (...a: unknown[]) => relayPairCodeMock(...a),
  },
}));

import { RelayPane } from "./RelayPane";

const STOPPED_LIVE = {
  serving: false,
  activeStreams: 0,
  blocking: null,
  lastError: null,
  upstreams: [],
};

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
    live: STOPPED_LIVE,
    ...over,
  };
}

function device(over: Partial<RelayDevice> = {}): RelayDevice {
  return {
    id: "d1",
    name: "studio",
    createdAt: "2026-10-01T00:00:00Z",
    revokedAt: null,
    includeFuture: false,
    grants: ["u1"],
    lastSeen: null,
    today: { requests: 0, failed: 0, costUsd: 0, unpriced: 0, familyEstimate: false },
    ...over,
  };
}

function publication(over: Partial<RelayPublication> = {}): RelayPublication {
  return {
    id: "u1",
    generation: 1,
    kind: "account",
    accountEmail: "spare@example.com",
    routeId: null,
    routeName: null,
    ...over,
  };
}

const SERVING_HERE = {
  serving: true,
  runningHere: true,
  live: { ...STOPPED_LIVE, serving: true },
} as const;

/** A promise the test settles by hand, to order replies. */
function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** Run pending microtasks and due timers under fake timers. */
const flush = (ms = 0) =>
  act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });

/** See RemotePane.test.tsx: the cast is confined to this helper, and a
 *  missing key resolves to itself so the assertion fails loudly. */
const t = (k: string, o?: Record<string, unknown>) =>
  i18n.t(k as never, { ns: "settings", ...o }) as string;

beforeEach(() => {
  for (const m of [
    relayStatusMock,
    relayStartMock,
    relayStopMock,
    relaySetBindMock,
    relayPublishMock,
    relayUnpublishMock,
    relayRevokeDeviceMock,
    relayPairCodeMock,
  ])
    m.mockReset();
});
afterEach(() => vi.useRealTimers());

describe("RelayPane", () => {
  it("states that quitting stops the relay and that TLS is always on", async () => {
    relayStatusMock.mockResolvedValue(status());
    render(<RelayPane />);
    expect(await screen.findByText(t("relay.quitStops"))).toBeInTheDocument();
    expect(screen.getByText(t("relay.tlsAlways"))).toBeInTheDocument();
  });

  it("discloses the terms risk and the copied-login rule before anything is published", async () => {
    relayStatusMock.mockResolvedValue(status());
    render(<RelayPane />);
    expect(await screen.findByText(t("relay.terms"))).toBeInTheDocument();
    expect(screen.getByText(t("relay.copiedLogin"))).toBeInTheDocument();
  });

  it("shows an ineligible account with its reason and no Publish button", async () => {
    relayStatusMock.mockResolvedValue(
      status({
        accounts: [
          { email: "active@example.com", ineligible: "cc_active" },
          { email: "spare@example.com", ineligible: null },
        ],
      }),
    );
    render(<RelayPane />);
    const active = (await screen.findByText("active@example.com")).closest("li")!;
    expect(within(active).getByText(t("relay.ineligible.cc_active"))).toBeInTheDocument();
    expect(within(active).queryByRole("button")).toBeNull();
    const spare = screen.getByText("spare@example.com").closest("li")!;
    expect(within(spare).getByRole("button", { name: t("relay.publish") })).toBeEnabled();
  });

  it("publishes the chosen account by email", async () => {
    relayStatusMock.mockResolvedValue(
      status({ accounts: [{ email: "spare@example.com", ineligible: null }] }),
    );
    relayPublishMock.mockResolvedValue("u1");
    render(<RelayPane />);
    await userEvent.click(await screen.findByRole("button", { name: t("relay.publish") }));
    await waitFor(() => expect(relayPublishMock).toHaveBeenCalledWith("spare@example.com"));
  });

  it("refuses changes, with the reason inline, while a terminal relay serves", async () => {
    relayStatusMock.mockResolvedValue(
      status({
        serving: true,
        runningHere: false,
        accounts: [{ email: "spare@example.com", ineligible: null }],
      }),
    );
    render(<RelayPane />);
    expect(await screen.findByText(t("relay.servingElsewhere"))).toBeInTheDocument();
    expect(screen.getByRole("button", { name: t("relay.start") })).toBeDisabled();
    expect(screen.getByText(t("relay.cannotStartElsewhere"))).toBeInTheDocument();
    expect(screen.getByRole("button", { name: t("relay.publish") })).toBeDisabled();
    expect(screen.getByRole("button", { name: t("relay.saveBind") })).toBeDisabled();
    expect(screen.getAllByText(t("relay.lockedReason")).length).toBeGreaterThan(0);
    // No Stop for a process this one cannot stop.
    expect(screen.queryByRole("button", { name: t("relay.stop") })).toBeNull();
  });

  it("offers Stop only when this process serves", async () => {
    relayStatusMock.mockResolvedValue(
      status({ serving: true, runningHere: true, addr: "127.0.0.1:8787", live: { ...STOPPED_LIVE, serving: true } }),
    );
    relayStopMock.mockResolvedValue(true);
    render(<RelayPane />);
    await userEvent.click(await screen.findByRole("button", { name: t("relay.stop") }));
    await waitFor(() => expect(relayStopMock).toHaveBeenCalled());
  });

  it("warns at the control when the bind is every interface", async () => {
    relayStatusMock.mockResolvedValue(status({ bind: "0.0.0.0", exposure: "every_interface" }));
    render(<RelayPane />);
    expect(await screen.findByText(t("relay.everyInterface"), { exact: false })).toBeInTheDocument();
  });

  it("disables pairing with a reason when nothing serves here", async () => {
    relayStatusMock.mockResolvedValue(status());
    render(<RelayPane />);
    expect(await screen.findByRole("button", { name: t("relay.pair") })).toBeDisabled();
    expect(screen.getByText(t("relay.cannotPairNotServing"))).toBeInTheDocument();
  });

  it("shows the pairing code, its expiry and the CA fingerprint", async () => {
    relayStatusMock.mockResolvedValue(
      status({ serving: true, runningHere: true, live: { ...STOPPED_LIVE, serving: true } }),
    );
    relayPairCodeMock.mockResolvedValue({ code: "ABCD-EFGH", expiresInSecs: 600, caFingerprint: "aa:bb" });
    render(<RelayPane />);
    await userEvent.click(await screen.findByRole("button", { name: t("relay.pair") }));
    expect(await screen.findByText("ABCD-EFGH")).toBeInTheDocument();
    expect(relayPairCodeMock).toHaveBeenCalledWith(false);
    // The literal English, not `t()` with the pane's own arguments: the
    // catalog is plural, and `{ minutes }` without `count` rendered the
    // raw key, which a `t()`-built expectation matched just as well.
    expect(screen.getByText("Expires in 10 minutes.")).toBeInTheDocument();
    expect(screen.getByText("aa:bb")).toBeInTheDocument();
  });

  it("revokes a device only after confirming", async () => {
    relayStatusMock.mockResolvedValue(status({ devices: [device()] }));
    relayRevokeDeviceMock.mockResolvedValue({ alreadyRevoked: false });
    render(<RelayPane />);
    await userEvent.click(await screen.findByRole("button", { name: t("relay.revoke") }));
    expect(relayRevokeDeviceMock).not.toHaveBeenCalled();
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: t("relay.revoke") }));
    await waitFor(() => expect(relayRevokeDeviceMock).toHaveBeenCalledWith("d1"));
  });

  it("marks today's spend as an estimate and omits it when zero", async () => {
    relayStatusMock.mockResolvedValue(
      status({
        devices: [
          device(),
          device({
            id: "d2",
            name: "mini",
            today: { requests: 3, failed: 0, costUsd: 1.5, unpriced: 0, familyEstimate: false },
          }),
        ],
      }),
    );
    render(<RelayPane />);
    const mini = (await screen.findByText("mini")).closest("li")!;
    expect(mini.textContent).toContain("≈ $1.50");
    const studio = screen.getByText("studio").closest("li")!;
    expect(studio.textContent).not.toContain("today");
  });

  it("says today's usage is unknown when the ledger could not be read", async () => {
    relayStatusMock.mockResolvedValue(status({ devices: [device()], usageUnavailable: true }));
    render(<RelayPane />);
    expect(await screen.findByText(/usage ledger could not be read/)).toBeTruthy();
  });

  it("hides revoked devices", async () => {
    relayStatusMock.mockResolvedValue(
      status({ devices: [device({ revokedAt: "2026-10-02T00:00:00Z" })] }),
    );
    render(<RelayPane />);
    expect(await screen.findByText(t("relay.noDevices"))).toBeInTheDocument();
    expect(screen.queryByText("studio")).toBeNull();
  });

  it("marks a mixed day's subtotal as the priced requests' and counts the unpriced", async () => {
    relayStatusMock.mockResolvedValue(
      status({
        devices: [device({ today: { requests: 5, failed: 0, costUsd: 1.5, unpriced: 2, familyEstimate: false } })],
      }),
    );
    render(<RelayPane />);
    const row = (await screen.findByText("studio")).closest("li")!;
    expect(row.textContent).toContain("today 5 requests: ≈ $1.50 for 3 priced, 2 unpriced");
  });

  it("labels an all-unpriced day as unpriced and never prices it", async () => {
    relayStatusMock.mockResolvedValue(
      status({
        devices: [device({ today: { requests: 2, failed: 0, costUsd: 0, unpriced: 2, familyEstimate: false } })],
      }),
    );
    render(<RelayPane />);
    const row = (await screen.findByText("studio")).closest("li")!;
    expect(row.textContent).toContain("today 2 requests, unpriced");
    expect(row.textContent).not.toContain("$");
  });

  it("names a route publication by its route, not as a missing account", async () => {
    relayStatusMock.mockResolvedValue(
      status({
        publications: [
          publication({ id: "ds", kind: "route", accountEmail: null, routeId: "r-1", routeName: "DeepSeek" }),
          publication({ id: "gone", kind: "route", accountEmail: null, routeId: "r-2", routeName: null }),
        ],
      }),
    );
    render(<RelayPane />);
    expect(await screen.findByText("Route: DeepSeek")).toBeInTheDocument();
    expect(screen.getByText("Route no longer exists")).toBeInTheDocument();
    expect(screen.queryByText("Account no longer registered")).toBeNull();
  });

  it("states a stopped publication's state in words and keeps its id behind the dev badge", async () => {
    relayStatusMock.mockResolvedValue(status({ publications: [publication({ id: "spare-1" })] }));
    render(<RelayPane />);
    const row = (await screen.findByText("spare@example.com", { selector: "li *" })).closest("li")!;
    expect(within(row).getByText("not serving")).toBeInTheDocument();
    expect(row.textContent).not.toContain("spare-1");
  });

  it("does not claim a terminal relay serves a publication whose state it cannot see", async () => {
    relayStatusMock.mockResolvedValue(
      status({ serving: true, runningHere: false, publications: [publication()] }),
    );
    render(<RelayPane />);
    const row = (await screen.findByText("spare@example.com", { selector: "li *" })).closest("li")!;
    expect(within(row).getByText(t("relay.upstreamElsewhere"))).toBeInTheDocument();
    expect(row.textContent).not.toContain("served from a terminal");
  });

  it("states why Unpublish is disabled, in the Published section", async () => {
    relayStatusMock.mockResolvedValue(status({ ...SERVING_HERE, publications: [publication()] }));
    render(<RelayPane />);
    const unpublish = await screen.findByRole("button", { name: t("relay.unpublish") });
    expect(unpublish).toBeDisabled();
    const section = unpublish.closest("section")!;
    expect(within(section).getByText(t("relay.lockedReason"))).toBeInTheDocument();
  });

  it("names a start warning's publication and keeps its slug behind the dev badge", async () => {
    relayStatusMock.mockResolvedValue(
      status({
        ...SERVING_HERE,
        publications: [publication({ id: "spare-1" })],
        warnings: [
          { upstream: "spare-1", code: "credential_unavailable" },
          { upstream: "gone-7", code: "account_missing" },
        ],
      }),
    );
    render(<RelayPane />);
    expect(
      await screen.findByText("spare@example.com is not serving: credential_unavailable", { exact: false }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(t("relay.startWarning", { name: t("relay.publicationGone"), code: "account_missing" }), {
        exact: false,
      }),
    ).toBeInTheDocument();
    const statusSection = screen.getByText(t("relay.statusHeading")).closest("section")!;
    expect(statusSection.textContent).not.toContain("spare-1");
    expect(statusSection.textContent).not.toContain("gone-7");
  });

  it("disables Revoke, with the reason inline, while a terminal relay serves with no revoke socket", async () => {
    relayStatusMock.mockResolvedValue(status({ serving: true, runningHere: false, devices: [device()] }));
    render(<RelayPane />);
    expect(await screen.findByRole("button", { name: t("relay.revoke") })).toBeDisabled();
    expect(screen.getByText(t("relay.cannotRevokeElsewhere"))).toBeInTheDocument();
  });

  it("revokes through a terminal relay's revoke socket", async () => {
    relayStatusMock.mockResolvedValue(
      status({ serving: true, runningHere: false, revokeSocketPresent: true, devices: [device()] }),
    );
    relayRevokeDeviceMock.mockResolvedValue({ alreadyRevoked: false });
    const pushToast = vi.fn();
    render(<RelayPane pushToast={pushToast} />);
    await userEvent.click(await screen.findByRole("button", { name: t("relay.revoke") }));
    expect(screen.queryByText(t("relay.cannotRevokeElsewhere"))).toBeNull();
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: t("relay.revoke") }));
    await waitFor(() => expect(pushToast).toHaveBeenCalledWith("info", t("relay.revokedOk")));
    expect(relayRevokeDeviceMock).toHaveBeenCalledWith("d1");
  });

  it("says so when the device was already revoked", async () => {
    relayStatusMock.mockResolvedValue(status({ ...SERVING_HERE, devices: [device()] }));
    relayRevokeDeviceMock.mockResolvedValue({ alreadyRevoked: true });
    const pushToast = vi.fn();
    render(<RelayPane pushToast={pushToast} />);
    await userEvent.click(await screen.findByRole("button", { name: t("relay.revoke") }));
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: t("relay.revoke") }));
    await waitFor(() => expect(pushToast).toHaveBeenCalledWith("info", t("relay.alreadyRevoked")));
    expect(pushToast).not.toHaveBeenCalledWith("info", t("relay.revokedOk"));
  });

  it("renders an unconfirmed revoke as maybe-revoked, not as a failure to change the relay", async () => {
    relayStatusMock.mockResolvedValue(
      status({ serving: true, runningHere: false, revokeSocketPresent: true, devices: [device()] }),
    );
    relayRevokeDeviceMock.mockRejectedValue({
      code: "relay.revoke_outcome_unknown",
      params: {},
      message: "the serving relay did not confirm the revocation",
    });
    const pushToast = vi.fn();
    render(<RelayPane pushToast={pushToast} />);
    await userEvent.click(await screen.findByRole("button", { name: t("relay.revoke") }));
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: t("relay.revoke") }));
    await waitFor(() =>
      expect(pushToast).toHaveBeenCalledWith(
        "error",
        `${t("relay.revokeScope")}: ${i18n.t("errors:relay.revoke_outcome_unknown")}`,
      ),
    );
  });

  it("notes, with its reason, that a terminal cannot revoke through the relay serving here", async () => {
    relayStatusMock.mockResolvedValue(
      status({ ...SERVING_HERE, liveRevokeUnavailable: "path_too_long", devices: [device()] }),
    );
    render(<RelayPane />);
    expect(
      await screen.findByText(
        t("relay.noLiveRevoke", { reason: t("relay.noLiveRevokeReason.path_too_long") }),
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: t("relay.revoke") })).toBeEnabled();
  });

  it("does not note the missing socket on a platform that has none", async () => {
    relayStatusMock.mockResolvedValue(
      status({ ...SERVING_HERE, liveRevokeUnavailable: "unsupported", devices: [device()] }),
    );
    render(<RelayPane />);
    await screen.findByRole("button", { name: t("relay.revoke") });
    expect(screen.queryByText(t("relay.noLiveRevokeReason.unsupported"), { exact: false })).toBeNull();
  });

  it("keeps Revoke enabled while this process serves, which revokes live", async () => {
    relayStatusMock.mockResolvedValue(status({ ...SERVING_HERE, devices: [device()] }));
    render(<RelayPane />);
    expect(await screen.findByRole("button", { name: t("relay.revoke") })).toBeEnabled();
    expect(screen.queryByText(t("relay.cannotRevokeElsewhere"))).toBeNull();
  });

  it("shows a skeleton of the pane, not a line of text, while the first read is pending", () => {
    relayStatusMock.mockReturnValue(new Promise(() => {}));
    render(<RelayPane />);
    expect(screen.getByRole("status")).toHaveAttribute("aria-busy", "true");
  });

  it("keeps the last state on screen when a later read fails", async () => {
    vi.useFakeTimers();
    relayStatusMock.mockResolvedValueOnce(status({ devices: [device()] })).mockRejectedValue(new Error("boom"));
    render(<RelayPane />);
    await flush();
    expect(screen.getByText("studio")).toBeInTheDocument();
    await flush(4000);
    expect(screen.getByRole("alert").textContent).toContain(t("relay.loadFailed"));
    expect(screen.getByText("studio")).toBeInTheDocument();
  });

  it("shows a store read error instead of the pane when the first read fails", async () => {
    // `relay_status` refuses a corrupt store file while another process
    // serves rather than reporting it empty.
    relayStatusMock.mockRejectedValue(new Error("relay devices store is unreadable"));
    render(<RelayPane />);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("relay devices store is unreadable");
    expect(screen.queryByRole("button", { name: t("relay.start") })).toBeNull();
  });

  describe("a change in flight", () => {
    it("labels the running Start and names why the address Save is off", async () => {
      const start = deferred<string>();
      relayStatusMock.mockResolvedValue(status());
      relayStartMock.mockReturnValue(start.promise);
      render(<RelayPane />);
      await userEvent.click(await screen.findByRole("button", { name: t("relay.start") }));
      expect(await screen.findByRole("button", { name: t("relay.starting") })).toBeDisabled();
      const save = screen.getByRole("button", { name: t("relay.saveBind") });
      expect(save).toBeDisabled();
      expect(within(save.closest("section")!).getByText(t("relay.busyReason"))).toBeInTheDocument();
      // The Start's own section needs no second reason: its label is it.
      const startSection = screen.getByText(t("relay.statusHeading")).closest("section")!;
      expect(within(startSection).queryByText(t("relay.busyReason"))).toBeNull();
      start.resolve("127.0.0.1:8787");
      await waitFor(() => expect(screen.queryByText(t("relay.busyReason"))).toBeNull());
    });

    it("states why Start, Publish and Unpublish are off while another change runs", async () => {
      const publish = deferred<string>();
      relayStatusMock.mockResolvedValue(
        status({
          accounts: [
            { email: "spare@example.com", ineligible: null },
            { email: "other@example.com", ineligible: null },
          ],
          publications: [publication({ accountEmail: "third@example.com" })],
        }),
      );
      relayPublishMock.mockReturnValue(publish.promise);
      render(<RelayPane />);
      const spare = (await screen.findByText("spare@example.com")).closest("li")!;
      await userEvent.click(within(spare).getByRole("button", { name: t("relay.publish") }));
      expect(await within(spare).findByRole("button", { name: t("relay.publishing") })).toBeDisabled();
      const other = screen.getByText("other@example.com").closest("li")!;
      expect(within(other).getByRole("button", { name: t("relay.publish") })).toBeDisabled();
      const unpublish = screen.getByRole("button", { name: t("relay.unpublish") });
      expect(unpublish).toBeDisabled();
      const start = screen.getByRole("button", { name: t("relay.start") });
      expect(start).toBeDisabled();
      for (const control of [other, unpublish, start]) {
        expect(within(control.closest("section")!).getByText(t("relay.busyReason"))).toBeInTheDocument();
      }
      publish.resolve("u2");
      await waitFor(() => expect(screen.queryByText(t("relay.busyReason"))).toBeNull());
    });
  });

  describe("ordering", () => {
    it("applies only the newest read when replies arrive reversed", async () => {
      vi.useFakeTimers();
      const first = deferred<RelayStatus>();
      const second = deferred<RelayStatus>();
      relayStatusMock.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
      render(<RelayPane />);
      await flush();
      fireEvent.keyDown(window, { key: "r", metaKey: true }); // ⌘R issues the second read
      await flush();
      expect(relayStatusMock).toHaveBeenCalledTimes(2);
      second.resolve(status(SERVING_HERE));
      await flush();
      first.resolve(status());
      await flush();
      expect(screen.getByText(t("relay.state.here"))).toBeInTheDocument();
      expect(screen.queryByText(t("relay.state.off"))).toBeNull();
    });

    it("discards a read that began before a change once the change lands", async () => {
      vi.useFakeTimers();
      const stale = deferred<RelayStatus>();
      relayStatusMock
        .mockResolvedValueOnce(status())
        .mockReturnValueOnce(stale.promise)
        .mockResolvedValue(status(SERVING_HERE));
      relayStartMock.mockResolvedValue("127.0.0.1:8787");
      render(<RelayPane />);
      await flush();
      await flush(4000); // a poll is now in flight, holding the pre-start state
      fireEvent.click(screen.getByRole("button", { name: t("relay.start") }));
      await flush();
      expect(screen.getByText(t("relay.state.here"))).toBeInTheDocument();
      stale.resolve(status());
      await flush();
      expect(screen.getByText(t("relay.state.here"))).toBeInTheDocument();
    });
    it("still updates when every read takes longer than the poll interval", async () => {
      vi.useFakeTimers();
      let reply = status();
      relayStatusMock.mockImplementation(
        () => new Promise<RelayStatus>((res) => setTimeout(() => res(reply), 5000)),
      );
      render(<RelayPane />);
      await flush(5000);
      expect(screen.getByText(t("relay.state.off"))).toBeInTheDocument();
      // The tick at 4 s found a read in flight and skipped.
      expect(relayStatusMock).toHaveBeenCalledTimes(1);
      reply = status(SERVING_HERE);
      await flush(8000); // tick at 8 s, reply at 13 s
      expect(screen.getByText(t("relay.state.here"))).toBeInTheDocument();
    });
  });

  describe("address fields", () => {
    it("does not refill a field the user cleared when the next poll lands", async () => {
      vi.useFakeTimers();
      relayStatusMock.mockResolvedValue(status());
      render(<RelayPane />);
      await flush();
      const bind = screen.getByRole("textbox", { name: t("relay.bind") });
      expect(bind).toHaveValue("127.0.0.1");
      fireEvent.change(bind, { target: { value: "" } });
      await flush(4000);
      expect(relayStatusMock).toHaveBeenCalledTimes(2);
      expect(bind).toHaveValue("");
    });

    it("saves the trimmed address and port, then re-reads", async () => {
      relayStatusMock.mockResolvedValue(status());
      relaySetBindMock.mockResolvedValue("private_network");
      const pushToast = vi.fn();
      render(<RelayPane pushToast={pushToast} />);
      const bind = await screen.findByRole("textbox", { name: t("relay.bind") });
      await userEvent.clear(bind);
      await userEvent.type(bind, " 192.168.1.5 ");
      const port = screen.getByRole("textbox", { name: t("relay.port") });
      await userEvent.clear(port);
      await userEvent.type(port, "9000");
      const reads = relayStatusMock.mock.calls.length;
      await userEvent.click(screen.getByRole("button", { name: t("relay.saveBind") }));
      await waitFor(() => expect(relaySetBindMock).toHaveBeenCalledWith("192.168.1.5", 9000));
      await waitFor(() => expect(relayStatusMock.mock.calls.length).toBeGreaterThan(reads));
      expect(pushToast).toHaveBeenCalledWith("info", t("relay.bindSaved"));
    });

    it("sends no port when the port field is empty", async () => {
      relayStatusMock.mockResolvedValue(status());
      relaySetBindMock.mockResolvedValue("loopback");
      render(<RelayPane />);
      await userEvent.clear(await screen.findByRole("textbox", { name: t("relay.port") }));
      await userEvent.click(screen.getByRole("button", { name: t("relay.saveBind") }));
      await waitFor(() => expect(relaySetBindMock).toHaveBeenCalledWith("127.0.0.1", undefined));
    });

    it.each(["abc", "0", "65536", "80.5", "1e3", "-1"])(
      "refuses port %j inline and never sends it",
      async (bad) => {
        relayStatusMock.mockResolvedValue(status());
        render(<RelayPane />);
        const port = await screen.findByRole("textbox", { name: t("relay.port") });
        fireEvent.change(port, { target: { value: bad } });
        expect(screen.getByText(t("relay.portInvalid"))).toBeInTheDocument();
        const save = screen.getByRole("button", { name: t("relay.saveBind") });
        expect(save).toBeDisabled();
        fireEvent.click(save);
        expect(relaySetBindMock).not.toHaveBeenCalled();
      },
    );

    it("says why Save is off when the address is blank", async () => {
      relayStatusMock.mockResolvedValue(status());
      render(<RelayPane />);
      const bind = await screen.findByRole("textbox", { name: t("relay.bind") });
      expect(screen.queryByText(t("relay.bindRequired"))).toBeNull();
      fireEvent.change(bind, { target: { value: "  " } });
      expect(screen.getByRole("button", { name: t("relay.saveBind") })).toBeDisabled();
      expect(screen.getByText(t("relay.bindRequired"))).toBeInTheDocument();
    });

    it("reports a refused address as an error toast", async () => {
      relayStatusMock.mockResolvedValue(status());
      relaySetBindMock.mockRejectedValue(new Error("8.8.8.8 is globally routable"));
      const pushToast = vi.fn();
      render(<RelayPane pushToast={pushToast} />);
      await userEvent.click(await screen.findByRole("button", { name: t("relay.saveBind") }));
      await waitFor(() =>
        expect(pushToast).toHaveBeenCalledWith("error", expect.stringContaining("globally routable")),
      );
      expect(pushToast).not.toHaveBeenCalledWith("info", expect.anything());
    });
  });

  describe("start and unpublish", () => {
    it("starts the relay, confirms it, and re-reads", async () => {
      relayStatusMock.mockResolvedValueOnce(status()).mockResolvedValue(status(SERVING_HERE));
      relayStartMock.mockResolvedValue("127.0.0.1:8787");
      const pushToast = vi.fn();
      render(<RelayPane pushToast={pushToast} />);
      await userEvent.click(await screen.findByRole("button", { name: t("relay.start") }));
      expect(relayStartMock).toHaveBeenCalledWith();
      expect(await screen.findByRole("button", { name: t("relay.stop") })).toBeInTheDocument();
      expect(pushToast).toHaveBeenCalledWith("info", t("relay.started"));
    });

    it("reports a refused start and leaves Start available", async () => {
      relayStatusMock.mockResolvedValue(status());
      relayStartMock.mockRejectedValue(new Error("relay config was recovered"));
      const pushToast = vi.fn();
      render(<RelayPane pushToast={pushToast} />);
      await userEvent.click(await screen.findByRole("button", { name: t("relay.start") }));
      await waitFor(() =>
        expect(pushToast).toHaveBeenCalledWith("error", expect.stringContaining("relay config was recovered")),
      );
      expect(screen.getByRole("button", { name: t("relay.start") })).toBeEnabled();
    });

    it("unpublishes by upstream id and re-reads", async () => {
      relayStatusMock.mockResolvedValueOnce(status({ publications: [publication()] })).mockResolvedValue(status());
      relayUnpublishMock.mockResolvedValue(undefined);
      const pushToast = vi.fn();
      render(<RelayPane pushToast={pushToast} />);
      await userEvent.click(await screen.findByRole("button", { name: t("relay.unpublish") }));
      expect(relayUnpublishMock).toHaveBeenCalledWith("u1");
      expect(await screen.findByText(t("relay.noPublications"))).toBeInTheDocument();
      expect(pushToast).toHaveBeenCalledWith("info", t("relay.unpublishedOk"));
    });

    it("reports a refused unpublish and keeps the row", async () => {
      relayStatusMock.mockResolvedValue(status({ publications: [publication()] }));
      relayUnpublishMock.mockRejectedValue(new Error("the relay is serving"));
      const pushToast = vi.fn();
      render(<RelayPane pushToast={pushToast} />);
      await userEvent.click(await screen.findByRole("button", { name: t("relay.unpublish") }));
      await waitFor(() =>
        expect(pushToast).toHaveBeenCalledWith("error", expect.stringContaining("the relay is serving")),
      );
      expect(screen.getByRole("button", { name: t("relay.unpublish") })).toBeEnabled();
    });
  });

  describe("pairing code expiry", () => {
    it("counts down and retires the code at its deadline", async () => {
      vi.useFakeTimers();
      relayStatusMock.mockResolvedValue(status(SERVING_HERE));
      relayPairCodeMock.mockResolvedValue({ code: "ABCD-EFGH", expiresInSecs: 120, caFingerprint: "aa:bb" });
      render(<RelayPane />);
      await flush();
      fireEvent.click(screen.getByRole("button", { name: t("relay.pair") }));
      await flush();
      expect(screen.getByText("ABCD-EFGH")).toBeInTheDocument();
      expect(screen.getByText("Expires in 2 minutes.")).toBeInTheDocument();
      await flush(61_000);
      expect(screen.getByText("Expires in 1 minute.")).toBeInTheDocument();
      await flush(60_000);
      expect(screen.queryByText("ABCD-EFGH")).toBeNull();
      expect(screen.getByText(t("relay.pairExpired"))).toBeInTheDocument();
    });

    it("counts the deadline from the request, so a late reply is not shown as live", async () => {
      vi.useFakeTimers();
      relayStatusMock.mockResolvedValue(status(SERVING_HERE));
      const reply = deferred<{ code: string; expiresInSecs: number; caFingerprint: string }>();
      relayPairCodeMock.mockReturnValue(reply.promise);
      render(<RelayPane />);
      await flush();
      fireEvent.click(screen.getByRole("button", { name: t("relay.pair") }));
      await flush(121_000); // delivery outlasted the code's lifetime
      reply.resolve({ code: "ABCD-EFGH", expiresInSecs: 120, caFingerprint: "aa:bb" });
      await flush();
      expect(screen.queryByText("ABCD-EFGH")).toBeNull();
      expect(screen.getByText(t("relay.pairExpired"))).toBeInTheDocument();
    });
  });
});
