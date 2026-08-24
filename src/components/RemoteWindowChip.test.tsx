import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { RemoteWindowChip } from "./RemoteWindowChip";

const peerInboundState = vi.fn();

vi.mock("../api", () => ({
  api: { peerInboundState: () => peerInboundState() },
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

const closed = {
  open: false,
  unmanagedOpen: false,
  remainingSecs: null,
  observed: "hold",
  recordRecovered: false,
};

describe("RemoteWindowChip", () => {
  beforeEach(() => {
    peerInboundState.mockReset();
  });

  it("renders nothing while the gate is shut", async () => {
    peerInboundState.mockResolvedValue(closed);
    const { container } = render(<RemoteWindowChip />);
    await waitFor(() => expect(peerInboundState).toHaveBeenCalled());
    expect(container.querySelector(".statusbar-chip")).toBeNull();
  });

  it("shows the countdown for a managed window", async () => {
    peerInboundState.mockResolvedValue({
      open: true,
      unmanagedOpen: false,
      remainingSecs: 725,
      observed: "accept",
      recordRecovered: false,
    });
    render(<RemoteWindowChip />);
    // 725s rounds up to 13 minutes.
    expect(await screen.findByText(/13m/)).toBeTruthy();
  });

  it("warns, without a timer, when nothing is minding an open gate", async () => {
    peerInboundState.mockResolvedValue({
      open: true,
      unmanagedOpen: true,
      remainingSecs: null,
      observed: "accept",
      recordRecovered: false,
    });
    render(<RemoteWindowChip />);
    const chip = await screen.findByRole("status");
    expect(chip.className).toContain("warn");
    expect(chip.textContent).not.toMatch(/\dm/);
    // The generic unmanaged wording, not the lost-record one.
    expect(chip.getAttribute("title") ?? "").not.toMatch(/unreadable/i);
  });

  it("names a lost grant record as the reason nothing will close the gate", async () => {
    // AGENTS.md calls the grant store fail-loud. `record_recovered`
    // reached the HTTP surface and stopped at the Tauri DTO, so on the
    // desktop the deadline could vanish with nothing naming the cause.
    peerInboundState.mockResolvedValue({
      open: true,
      unmanagedOpen: true,
      remainingSecs: null,
      observed: "accept",
      recordRecovered: true,
    });
    render(<RemoteWindowChip />);
    const chip = await screen.findByRole("status");
    expect(chip.className).toContain("warn");
    expect(chip.getAttribute("title") ?? "").toMatch(/unreadable|reset/i);
  });

  it("a failed read is NOT rendered as a closed gate", async () => {
    // The defect this locks down: the catch set state to `null`, and a
    // closed gate also renders `null`, so "I could not read it" and
    // "it is shut" were pixel-identical on a security indicator.
    peerInboundState.mockRejectedValue(new Error("ipc down"));
    render(<RemoteWindowChip />);
    const chip = await screen.findByRole("status");
    expect(chip.className).toContain("warn");
    expect(chip.getAttribute("title") ?? "").toMatch(/could not read/i);
  });

});
