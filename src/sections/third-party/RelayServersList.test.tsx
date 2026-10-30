import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";

import { i18n } from "../../lib/i18n";
import type { RelayClientServer, RelayClientUpstream } from "../../api";
import { RelayServersList } from "./RelayServersList";

const t = (k: string, o?: Record<string, unknown>) =>
  i18n.t(k as never, { ns: "providers", ...o }) as string;

const upstream = (u: Partial<RelayClientUpstream> & { id: string }): RelayClientUpstream => ({
  eligible: false,
  tier: null,
  tierModel: null,
  ttfbMs: null,
  scoreMs: null,
  reason: null,
  cooldownSecs: null,
  saturatedWindow: null,
  saturatedUtilization: null,
  ...u,
});

const studio: RelayClientServer = {
  serverId: "srv-internal-7f3a",
  name: "studio",
  baseUrl: "https://studio.example:8443",
  reachable: true,
  error: null,
  rttMs: 12,
  defaultUpstream: "max",
  upstreams: [
    upstream({
      id: "max",
      eligible: true,
      tier: "measured",
      tierModel: "claude-opus-4",
      ttfbMs: 400,
      scoreMs: 412,
    }),
    upstream({ id: "pro", reason: "cooldown", cooldownSecs: 42 }),
    upstream({ id: "team", reason: "not_accepted" }),
  ],
};

const attic: RelayClientServer = {
  serverId: "srv-internal-9b1c",
  name: "attic",
  baseUrl: "https://attic.example:8443",
  reachable: false,
  error: "not_paired",
  rttMs: null,
  defaultUpstream: null,
  upstreams: [upstream({ id: "sub", reason: "unreachable" })],
};

describe("RelayServersList", () => {
  it("resolves its strings from the catalog, so the assertions below are not key-against-key", () => {
    expect(t("relayServers.reachableRtt", { ms: 12 })).toBe("Reachable · 12 ms");
    expect(t("relayServers.probe.not_paired")).not.toBe("relayServers.probe.not_paired");
  });

  it("shows a skeleton while the first probe is in flight", () => {
    render(<RelayServersList servers={null} error={null} />);
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("with no paired servers, points at the pairing command and offers no control", () => {
    render(<RelayServersList servers={[]} error={null} />);
    expect(screen.getByText(t("relayServers.empty"), { exact: false })).toBeInTheDocument();
    expect(screen.getByText("claudepot relay pair <host> --code <code>")).toBeInTheDocument();
    expect(screen.queryAllByRole("button")).toHaveLength(0);
    expect(screen.queryByRole("listitem")).toBeNull();
  });

  it("lists a reachable server with its eligible and ineligible upstreams", () => {
    render(<RelayServersList servers={[studio]} error={null} />);
    const row = screen.getByRole("heading", { name: "studio" }).closest("li")!;
    const r = within(row);
    expect(r.getByText(t("relayServers.reachableRtt", { ms: 12 }))).toBeInTheDocument();
    expect(r.getByText("https://studio.example:8443")).toHaveAttribute(
      "title",
      "https://studio.example:8443",
    );

    const ups = within(r.getByRole("list", { name: t("relayServers.upstreamsLabel", { name: "studio" }) }));
    const [max, pro, team] = ups.getAllByRole("listitem");
    expect(within(max).getByText(t("relayServers.default"))).toBeInTheDocument();
    expect(within(max).getByText(t("relayServers.ready"))).toBeInTheDocument();
    expect(
      within(max).getByText(
        t("relayServers.tierMeasured", { model: "claude-opus-4", ttfb: 400, score: 412 }),
      ),
    ).toBeInTheDocument();
    expect(within(pro).queryByText(t("relayServers.default"))).toBeNull();
    expect(within(pro).getByText(t("relayServers.notReady"))).toBeInTheDocument();
    expect(
      within(pro).getByText(t("relayServers.reason.cooldown", { secs: 42 })),
    ).toBeInTheDocument();
    expect(within(team).getByText("claudepot relay accept")).toBeInTheDocument();

    // The internal server id never reaches the screen.
    expect(document.body.textContent).not.toContain("srv-internal");
    // The run note names the CLI launch path.
    expect(screen.getByText("claudepot relay run")).toBeInTheDocument();
  });

  it("marks an unreachable server and states the probe failure from its code", () => {
    render(<RelayServersList servers={[studio, attic]} error={null} />);
    const row = within(screen.getByRole("heading", { name: "attic" }).closest("li")!);
    expect(row.getByText(t("relayServers.unreachable"))).toBeInTheDocument();
    expect(row.getByText(t("relayServers.probe.not_paired"))).toBeInTheDocument();
    expect(row.getByText(t("relayServers.reason.unreachable"))).toBeInTheDocument();
    expect(row.queryByText(/ ms$/)).toBeNull();
  });

  it("states a load failure instead of an empty list", () => {
    render(<RelayServersList servers={null} error="relay-servers.json is corrupt" />);
    expect(screen.getByRole("alert")).toHaveTextContent("relay-servers.json is corrupt");
    expect(screen.queryByText(t("relayServers.empty"), { exact: false })).toBeNull();
  });
});
