import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { RetentionReport } from "../../api/cc-retention";
import { i18n } from "../../lib/i18n";

const retentionReportMock = vi.fn();
const retentionSetMock = vi.fn();
const retentionClearMock = vi.fn();

// No `retentionDisablePersistence` — the command was removed when CC
// 2.1.233 started rejecting `cleanupPeriodDays: 0`. Deliberately not
// stubbed: if the pane ever calls it again, this mock object is missing
// the method and the test fails loudly rather than silently passing.
vi.mock("../../api", () => ({
  api: {
    retentionReport: (...a: unknown[]) => retentionReportMock(...a),
    retentionSet: (...a: unknown[]) => retentionSetMock(...a),
    retentionClear: (...a: unknown[]) => retentionClearMock(...a),
  },
}));

import { RetentionPane } from "./RetentionPane";

function report(over: {
  state?: Partial<RetentionReport["state"]>;
  risk?: Partial<RetentionReport["risk"]>;
  swept?: Partial<RetentionReport["swept_elsewhere"]>;
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
    swept_elsewhere: {
      dirs: [],
      scan_incomplete: false,
      cache_dirs_skipped: 7,
      ...over.swept,
    },
  };
}

describe("RetentionPane", () => {
  beforeEach(() => {
    retentionReportMock.mockReset();
    retentionSetMock.mockReset();
    retentionClearMock.mockReset();
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
      await screen.findByText(/no saved conversations are scheduled for deletion/i),
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

  // `cleanupPeriodDays` is a global TTL over ~20 directories under
  // ~/.claude (verified against the 2.1.233 binary), but this pane
  // counts only conversations. Unscoped reassurance would therefore be
  // false: plenty may be scheduled for deletion that we did not count.
  it("scopes its reassurance to conversations and says what it omits", async () => {
    retentionReportMock.mockResolvedValue(report());
    render(<RetentionPane pushToast={toast()} />);
    expect(
      await screen.findByText(/no saved conversations are scheduled for deletion/i),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/not only a transcript setting/i),
    ).toBeInTheDocument();
    // The unscoped claim must not survive anywhere on the surface.
    expect(screen.queryByText(/^Nothing is scheduled for deletion\.$/)).toBeNull();
  });

  // The gap the scope note names in prose, now quantified. Counting
  // conversations alone and saying nothing about the rest was the
  // smaller version of the same over-claim.
  it("quantifies the other directories on the same timer", async () => {
    retentionReportMock.mockResolvedValue(
      report({
        swept: {
          dirs: [
            { rel: "file-history", what: "file edit history", kind: "content", entries: 412, already_deletable: 91 },
            { rel: "uploads", what: "files you uploaded", kind: "content", entries: 7, already_deletable: 0 },
          ],
          cache_dirs_skipped: 7,
        },
      }),
    );
    render(<RetentionPane pushToast={toast()} />);
    expect(await screen.findByText(/file edit history/i)).toBeInTheDocument();
    expect(screen.getByText(/91 past the cutoff/i)).toBeInTheDocument();
    // The directories it chose NOT to count are stated, not implied.
    expect(screen.getByText(/7 further directories/i)).toBeInTheDocument();
  });

  // design.md render-if-nonzero: a machine with nothing else on the
  // timer must not grow an empty section.
  it("omits the swept panel entirely when nothing else is on the timer", async () => {
    retentionReportMock.mockResolvedValue(report());
    render(<RetentionPane pushToast={toast()} />);
    await screen.findByRole("button", { name: "30 days" });
    expect(screen.queryByText(/Also on this timer/i)).toBeNull();
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

  // CC 2.1.233 rejects `cleanupPeriodDays: 0` and offers no
  // settings-level way to stop persisting, so the control that used to
  // write it is gone. This replaces "gates disabling persistence behind
  // a type-to-confirm": the gate was correct for a capability that no
  // longer exists.
  it("offers no way to stop saving transcripts, and says why", async () => {
    retentionReportMock.mockResolvedValue(report());
    render(<RetentionPane pushToast={toast()} />);
    await screen.findByRole("button", { name: "30 days" });
    expect(
      screen.queryByRole("button", { name: /stop saving/i }),
    ).toBeNull();
    // Absence alone is a worse outcome than an explanation: a user who
    // set this once will come looking for it.
    expect(
      screen.getByText(/no longer any way to stop saving transcripts/i),
    ).toBeInTheDocument();
  });

  // A `0` written by an older Claudepot is still on disk for anyone who
  // used the old control. It now suppresses cleanup rather than
  // disabling persistence — the opposite of what they chose.
  it("explains a legacy zero rather than reporting persistence as off", async () => {
    retentionReportMock.mockResolvedValue(
      report({
        state: {
          mode: "legacy_zero",
          configured_days: 0,
          effective_days: 30,
          is_cc_default: false,
          cleanup_suppressed: true,
        },
      }),
    );
    render(<RetentionPane pushToast={toast()} />);
    expect(
      await screen.findByText(/no longer accepts/i),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/transcripts are being written/i),
    ).toBeInTheDocument();
  });

  // While cleanup is suppressed the transcripts are protected by
  // accident, so a preset is not a preference — it re-arms deletion of
  // the whole backlog in one tap.
  it("confirms before a preset re-arms deletion on a suppressed setting", async () => {
    retentionReportMock.mockResolvedValue(
      report({
        state: {
          mode: "legacy_zero",
          configured_days: 0,
          effective_days: 30,
          is_cc_default: false,
          cleanup_suppressed: true,
        },
      }),
    );
    retentionSetMock.mockResolvedValue(report());
    const user = userEvent.setup();
    render(<RetentionPane pushToast={toast()} />);

    await user.click(await screen.findByRole("button", { name: "30 days" }));
    expect(retentionSetMock).not.toHaveBeenCalled();

    await user.click(
      screen.getByRole("button", { name: /re-enable deletion/i }),
    );
    await waitFor(() => expect(retentionSetMock).toHaveBeenCalledWith(30));
  });

  // ...and the confirmation is scoped to that state only. An ordinary
  // preset change must stay one click.
  it("applies a preset immediately when cleanup is not suppressed", async () => {
    retentionReportMock.mockResolvedValue(report());
    retentionSetMock.mockResolvedValue(report());
    render(<RetentionPane pushToast={toast()} />);
    await userEvent
      .setup()
      .click(await screen.findByRole("button", { name: "90 days" }));
    await waitFor(() => expect(retentionSetMock).toHaveBeenCalledWith(90));
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
    expect(
      await screen.findByText(/no saved conversations are scheduled for deletion/i),
    ).toBeInTheDocument();
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
    // The load-bearing half: never point a suppressed user at "restore
    // the default", which clears the error and re-arms deletion.
    expect(
      await screen.findByText(/rather than restoring the default/i),
    ).toBeInTheDocument();
    // And it must not assert WHY cleanup is suppressed. `resolve_retention`
    // maps an unreadable settings file to `Invalid` too, so "this value is
    // invalid" is a claim the pane may not have evidence for.
    expect(
      screen.getByText(/could not be read — Claudepot cannot tell which/i),
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

// The risk lines state how much history is about to be destroyed, so a
// grammar or resolution bug here is not cosmetic. All three keys used to
// pass `num` alone: `deletable` has `_one`/`_other`, and i18next selects
// plurals on `count`, so it never resolved and rendered the literal key
// `retention.risk.deletable` to the user — in English too. `horizon` and
// `totalOnMachine` had no plural forms at all ("1 transcripts on this
// machine"). `count` must be the raw number; `num` carries the grouped
// display form.
describe("retention risk lines — plural resolution", () => {
  const t = i18n.getFixedT("en", "settings");

  it("never renders a raw key", () => {
    for (const key of [
      "retention.risk.deletable",
      "retention.risk.horizon",
      "retention.risk.totalOnMachine",
    ] as const) {
      for (const n of [0, 1, 2]) {
        const out = t(key, { count: n, num: String(n), days: 30 });
        expect(out, `${key} @ ${n}`).not.toContain("retention.risk");
      }
    }
  });

  it("agrees with its count in English", () => {
    expect(t("retention.risk.deletable", { count: 1, num: "1" })).toContain(
      "1 transcript will",
    );
    expect(t("retention.risk.deletable", { count: 2, num: "2" })).toContain(
      "2 transcripts will",
    );
    expect(t("retention.risk.totalOnMachine", { count: 1, num: "1" })).toBe(
      "1 transcript on this machine.",
    );
    expect(t("retention.risk.horizon", { count: 1, num: "1", days: 30 })).toContain(
      "1 more crosses",
    );
  });

  it("resolves in zh-CN too", () => {
    const zh = i18n.getFixedT("zh-CN", "settings");
    for (const n of [1, 5]) {
      expect(zh("retention.risk.deletable", { count: n, num: String(n) })).toContain(
        "对话记录",
      );
    }
  });
});
