import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import type { DaemonStatus } from "../api/cc-daemon";

const ccDaemonStatus = vi.fn<() => Promise<DaemonStatus>>();
vi.mock("../api", () => ({ api: { ccDaemonStatus: () => ccDaemonStatus() } }));

// Imported after the mock so the hook binds to it.
const { SidebarBgBadge } = await import("./SidebarBgBadge");

function status(over: Partial<DaemonStatus> = {}): DaemonStatus {
  return {
    bgWorkers: over.bgWorkers ?? null,
    rosterPath: over.rosterPath ?? "/home/u/.claude/daemon/roster.json",
    parseStatus: over.parseStatus ?? { kind: "ok" },
  };
}

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  ccDaemonStatus.mockReset();
});
afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

describe("SidebarBgBadge", () => {
  it("renders the live worker count", async () => {
    ccDaemonStatus.mockResolvedValue(status({ bgWorkers: 3 }));
    render(<SidebarBgBadge />);
    await waitFor(() => expect(screen.getByText("3 bg workers")).toBeTruthy());
  });

  it("pluralizes a single worker", async () => {
    ccDaemonStatus.mockResolvedValue(status({ bgWorkers: 1 }));
    render(<SidebarBgBadge />);
    await waitFor(() => expect(screen.getByText("1 bg worker")).toBeTruthy());
  });

  // Render-if-nonzero: an idle daemon is not a row worth spending.
  it("renders nothing when the daemon is idle", async () => {
    ccDaemonStatus.mockResolvedValue(status({ bgWorkers: 0 }));
    const { container } = render(<SidebarBgBadge />);
    await waitFor(() => expect(ccDaemonStatus).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  // `null` is "couldn't tell", which must read as no signal — never
  // as a zero the user could mistake for a measured idle, and never
  // as a badge showing NaN.
  it("renders nothing when the count is unknown", async () => {
    ccDaemonStatus.mockResolvedValue(
      status({ bgWorkers: null, parseStatus: { kind: "degraded", reason: "proto 2" } }),
    );
    const { container } = render(<SidebarBgBadge />);
    await waitFor(() => expect(ccDaemonStatus).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  // The hook keeps the last good snapshot across a transient failure,
  // so a blip must not flicker a live badge off.
  it("keeps the last good count when a later read degrades", async () => {
    ccDaemonStatus
      .mockResolvedValueOnce(status({ bgWorkers: 2 }))
      .mockResolvedValue(status({ bgWorkers: null, parseStatus: { kind: "failed", reason: "gone" } }));
    render(<SidebarBgBadge />);
    await waitFor(() => expect(screen.getByText("2 bg workers")).toBeTruthy());
    await vi.advanceTimersByTimeAsync(60_000);
    await waitFor(() => expect(ccDaemonStatus).toHaveBeenCalledTimes(2));
    expect(screen.getByText("2 bg workers")).toBeTruthy();
  });

  // Collapsed rail: icon + bare number, with the phrase on the tooltip
  // since the column carries no text at that width.
  it("shows the count and a tooltip when collapsed", async () => {
    ccDaemonStatus.mockResolvedValue(status({ bgWorkers: 4 }));
    render(<SidebarBgBadge collapsed />);
    await waitFor(() => expect(screen.getByLabelText("4 bg workers")).toBeTruthy());
    expect(screen.getByText("4")).toBeTruthy();
  });
});
