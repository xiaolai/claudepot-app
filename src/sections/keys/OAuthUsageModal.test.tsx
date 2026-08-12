/**
 * OAuthUsageModal — model-scoped usage rows.
 *
 * Anthropic moved per-model windows out of the `seven_day_<model>`
 * keys (now null on every observed account) into a generic `limits[]`
 * array. These cover the rows built from that array; the plan-level
 * rows above them are unchanged.
 *
 * `UsageBody` is exercised directly rather than through the modal
 * wrapper — the wrapper adds only chrome and a data fetch.
 */
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import type { AccountUsage } from "../../types";
import { UsageBody } from "./OAuthUsageModal";

const base: AccountUsage = {
  five_hour: { utilization: 31, resets_at: null },
  seven_day: { utilization: 13, resets_at: null },
  seven_day_opus: null,
  seven_day_sonnet: null,
  seven_day_oauth_apps: null,
  seven_day_cowork: null,
  extra_usage: null,
};

describe("OAuthUsageModal model-scoped rows", () => {
  it("renders a row per model-scoped limit", () => {
    render(
      <UsageBody
        usage={{
          ...base,
          scoped_limits: [{ label: "Fable", utilization: 80, resets_at: null }],
        }}
      />,
    );
    expect(screen.getByText("7-day · Fable")).toBeInTheDocument();
  });

  it("renders none when the server reports none", () => {
    render(<UsageBody usage={{ ...base, scoped_limits: [] }} />);
    expect(screen.queryByText(/Fable/)).not.toBeInTheDocument();
  });
});
