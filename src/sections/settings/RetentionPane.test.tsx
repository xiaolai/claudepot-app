import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { RetentionReport } from "../../api/cc-retention";

const retentionReportMock = vi.fn();
const retentionSetMock = vi.fn();
const retentionClearMock = vi.fn();
const retentionDisableMock = vi.fn();

vi.mock("../../api", () => ({
  api: {
    retentionReport: (...a: unknown[]) => retentionReportMock(...a),
    retentionSet: (...a: unknown[]) => retentionSetMock(...a),
    retentionClear: (...a: unknown[]) => retentionClearMock(...a),
    retentionDisablePersistence: (...a: unknown[]) => retentionDisableMock(...a),
  },
}));

import { RetentionPane } from "./RetentionPane";

function report(over: {
  state?: Partial<RetentionReport["state"]>;
  risk?: Partial<RetentionReport["risk"]>;
} = {}): RetentionReport {
  return {
    state: {
      mode: "cc_default",
      configured_days: null,
      effective_days: 30,
      is_cc_default: true,
      cleanup_suppressed: false,
      ...over.state,
    },
    risk: {
      total_transcripts: 460,
      already_deletable: 0,
      at_risk_within_horizon: 0,
      oldest_ms: Date.UTC(2026, 5, 6),
      nested_immortal: 0,
      horizon_days: 7,
      scan_incomplete: false,
      ...over.risk,
    },
    is_durable_archive: false,
  };
}

describe("RetentionPane", () => {
  beforeEach(() => {
    retentionReportMock.mockReset();
    retentionSetMock.mockReset();
    retentionClearMock.mockReset();
    retentionDisableMock.mockReset();
  });
  afterEach(() => vi.restoreAllMocks());

  const toast = () => vi.fn();

  it("names the default as something the user did not choose", async () => {
    retentionReportMock.mockResolvedValue(report());
    render(<RetentionPane pushToast={toast()} />);
    expect(
      await screen.findByText(/not a choice you made/i),
    ).toBeInTheDocument();
  });

  it("leads with the count that will actually be deleted", async () => {
    retentionReportMock.mockResolvedValue(
      report({ risk: { already_deletable: 1247, at_risk_within_horizon: 30 } }),
    );
    render(<RetentionPane pushToast={toast()} />);
    expect(
      await screen.findByText(/1,247 transcripts will be deleted/i),
    ).toBeInTheDocument();
    expect(screen.getByText(/30 more cross the cutoff/i)).toBeInTheDocument();
  });

  // design.md render-if-nonzero: a quiet state must not show "0 will be
  // deleted · 0 at risk".
  it("collapses to a single line when nothing is at risk", async () => {
    retentionReportMock.mockResolvedValue(
      report({ state: { mode: "explicit", configured_days: 3650, effective_days: 3650, is_cc_default: false } }),
    );
    render(<RetentionPane pushToast={toast()} />);
    expect(
      await screen.findByText(/Nothing is scheduled for deletion/i),
    ).toBeInTheDocument();
    expect(screen.queryByText(/will be deleted the next time/i)).toBeNull();
    expect(screen.queryByText(/cross the cutoff/i)).toBeNull();
  });

  it("explains why disk usage cannot reveal the loss", async () => {
    retentionReportMock.mockResolvedValue(
      report({ risk: { nested_immortal: 9291 } }),
    );
    render(<RetentionPane pushToast={toast()} />);
    expect(
      await screen.findByText(/disk usage cannot reveal this loss/i),
    ).toBeInTheDocument();
  });

  it("says a longer window is not a backup", async () => {
    retentionReportMock.mockResolvedValue(report());
    render(<RetentionPane pushToast={toast()} />);
    expect(await screen.findByText(/is not a backup/i)).toBeInTheDocument();
  });

  it("writes the chosen preset", async () => {
    retentionReportMock.mockResolvedValue(report());
    retentionSetMock.mockResolvedValue(
      report({ state: { mode: "explicit", configured_days: 365, effective_days: 365, is_cc_default: false } }),
    );
    const push = vi.fn();
    render(<RetentionPane pushToast={push} />);
    await userEvent.setup().click(await screen.findByRole("button", { name: "1 year" }));
    await waitFor(() => expect(retentionSetMock).toHaveBeenCalledWith(365));
    expect(push).toHaveBeenCalledWith("info", expect.stringMatching(/1 year/i));
  });

  // The core rule: 0 is never a stop on the duration scale.
  it("offers no preset that would disable persistence", async () => {
    retentionReportMock.mockResolvedValue(report());
    render(<RetentionPane pushToast={toast()} />);
    await screen.findByRole("button", { name: "30 days" });
    for (const label of ["0", "0 days", "Off", "Never", "Disabled"]) {
      expect(screen.queryByRole("button", { name: label })).toBeNull();
    }
  });

  it("gates disabling persistence behind a type-to-confirm", async () => {
    retentionReportMock.mockResolvedValue(report());
    render(<RetentionPane pushToast={toast()} />);
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: /stop saving transcripts entirely/i }),
    );
    // Dialog is up, but the destructive call must not fire until the
    // phrase is typed.
    const confirm = await screen.findByRole("button", {
      name: /stop saving and delete/i,
    });
    await user.click(confirm);
    expect(retentionDisableMock).not.toHaveBeenCalled();

    await user.type(screen.getByRole("textbox"), "delete my transcripts");
    await user.click(screen.getByRole("button", { name: /stop saving and delete/i }));
    await waitFor(() => expect(retentionDisableMock).toHaveBeenCalled());
  });

  it("confirms before restoring the default that re-arms deletion", async () => {
    retentionReportMock.mockResolvedValue(
      report({ state: { mode: "explicit", configured_days: 3650, effective_days: 3650, is_cc_default: false } }),
    );
    retentionClearMock.mockResolvedValue(report());
    const user = userEvent.setup();
    render(<RetentionPane pushToast={toast()} />);
    await user.click(
      await screen.findByRole("button", { name: /restore claude code's default/i }),
    );
    expect(retentionClearMock).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: /^restore default$/i }));
    await waitFor(() => expect(retentionClearMock).toHaveBeenCalled());
  });

  it("reports an invalid configured value rather than trusting it", async () => {
    retentionReportMock.mockResolvedValue(
      report({ state: { mode: "invalid", configured_days: -1, effective_days: 30, is_cc_default: false } }),
    );
    render(<RetentionPane pushToast={toast()} />);
    expect(await screen.findByText(/Invalid value/i)).toBeInTheDocument();
  });

  // The worst failure mode: an unreadable tree rendering as "all clear".
  it("never reassures when the scan was incomplete", async () => {
    retentionReportMock.mockResolvedValue(
      report({ risk: { scan_incomplete: true } }),
    );
    render(<RetentionPane pushToast={toast()} />);
    expect(await screen.findByText(/counts are a floor, not a total/i)).toBeInTheDocument();
    expect(screen.queryByText(/Nothing is scheduled for deletion/i)).toBeNull();
  });

  // render-if-nonzero — "0 transcripts on this machine" must not ship.
  it("omits the transcript count when there are none", async () => {
    retentionReportMock.mockResolvedValue(
      report({ risk: { total_transcripts: 0 } }),
    );
    render(<RetentionPane pushToast={toast()} />);
    expect(await screen.findByText(/Nothing is scheduled for deletion/i)).toBeInTheDocument();
    expect(screen.queryByText(/0 transcripts on this machine/i)).toBeNull();
  });

  // Restoring the default here would clear the validation error and
  // re-arm deletion — the pane must say "fix the value" instead.
  it("warns not to restore the default when cleanup is suppressed", async () => {
    retentionReportMock.mockResolvedValue(
      report({
        state: {
          mode: "invalid",
          configured_days: -1,
          is_cc_default: false,
          cleanup_suppressed: true,
        },
      }),
    );
    render(<RetentionPane pushToast={toast()} />);
    expect(
      await screen.findByText(/correct the value rather than restoring the default/i),
    ).toBeInTheDocument();
  });

  it("offers a retry instead of hanging on Loading when the load fails", async () => {
    retentionReportMock.mockRejectedValueOnce(new Error("nope"));
    const push = vi.fn();
    render(<RetentionPane pushToast={push} />);
    const retry = await screen.findByRole("button", { name: /try again/i });
    expect(screen.queryByText(/^Loading…$/)).toBeNull();
    retentionReportMock.mockResolvedValue(report());
    await userEvent.setup().click(retry);
    await waitFor(() =>
      expect(screen.getByText(/not a choice you made/i)).toBeInTheDocument(),
    );
  });

  it("toasts and keeps state when a write fails", async () => {
    retentionReportMock.mockResolvedValue(report());
    retentionSetMock.mockRejectedValue(new Error("boom"));
    const push = vi.fn();
    render(<RetentionPane pushToast={push} />);
    await userEvent.setup().click(await screen.findByRole("button", { name: "90 days" }));
    await waitFor(() =>
      expect(push).toHaveBeenCalledWith("error", expect.stringContaining("boom")),
    );
  });
});
