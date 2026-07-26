import { describe, expect, it } from "vitest";
import {
  formatResetTime,
  hasUnreportedWindow,
  RENDERED_USAGE_WINDOWS,
} from "./format";
import type { UsageWindow } from "../../types";

const win = (resets_at: string | null, utilization = 0): UsageWindow =>
  ({ utilization, resets_at }) as UsageWindow;

/**
 * `hasUnreportedWindow` is the gate on "Wake windows" — the one action
 * in Claudepot that deliberately spends the user's plan quota. A false
 * positive offers to spend money for a visible change that won't
 * happen; a false negative hides the only way to populate the row.
 */
describe("hasUnreportedWindow", () => {
  it("is true when a rendered window has no reset to report", () => {
    // The idle-account case: server sends resets_at: null, card shows "—".
    expect(
      hasUnreportedWindow({ five_hour: win(null), seven_day: win(null) }),
    ).toBe(true);
  });

  it("is true when only one of the two rendered windows is unreported", () => {
    // Observed on a real account: 5h idle, 7d active.
    expect(
      hasUnreportedWindow({
        five_hour: win(null),
        seven_day: win("2026-07-31T09:00:00Z", 97),
      }),
    ).toBe(true);
  });

  it("is false when both rendered windows already report", () => {
    // Waking here would spend quota and change nothing on screen.
    expect(
      hasUnreportedWindow({
        five_hour: win("2026-07-26T03:30:00Z", 11),
        seven_day: win("2026-07-28T05:00:00Z", 73),
      }),
    ).toBe(false);
  });

  it("is false when usage is absent entirely", () => {
    // No usage yet (fetch in flight, expired token, no credentials).
    // Offering a wake here would be guessing.
    expect(hasUnreportedWindow(null)).toBe(false);
    expect(hasUnreportedWindow(undefined)).toBe(false);
  });

  it("scans every window UsageBlock renders, not just the first two", () => {
    // Regression: the gate originally checked only five_hour and
    // seven_day while UsageBlock renders six windows, so an account
    // showing "—" on the 7d Opus row had the wake action hidden.
    for (const key of RENDERED_USAGE_WINDOWS) {
      const usage: Record<string, ReturnType<typeof win>> = {
        five_hour: win("2026-07-26T03:30:00Z", 11),
        seven_day: win("2026-07-28T05:00:00Z", 73),
      };
      usage[key] = win(null);
      expect(hasUnreportedWindow(usage), `${key} should be scanned`).toBe(true);
    }
  });

  it("covers exactly the windows UsageBlock renders", () => {
    // If UsageBlock grows a row, this list must grow with it — that
    // drift is the bug the previous test documents.
    expect([...RENDERED_USAGE_WINDOWS]).toEqual([
      "five_hour",
      "seven_day",
      "seven_day_sonnet",
      "seven_day_opus",
      "seven_day_oauth_apps",
      "seven_day_cowork",
    ]);
  });

  it("treats a missing window as nothing to wake, not as unreported", () => {
    // Absent ≠ present-but-null. A window the plan doesn't have can
    // never be woken.
    expect(hasUnreportedWindow({ five_hour: null, seven_day: null })).toBe(
      false,
    );
  });
});

describe("formatResetTime", () => {
  it("renders an em-dash when there is no reset timestamp", () => {
    // This is the "—" the user asked about: honest, not missing data.
    expect(formatResetTime(null)).toBe("—");
    expect(formatResetTime(undefined)).toBe("—");
    expect(formatResetTime("")).toBe("—");
  });

  it("renders 'due' for a reset already in the past", () => {
    expect(formatResetTime("2020-01-01T00:00:00Z")).toBe("due");
  });

  it("renders minutes for a reset inside the hour", () => {
    const soon = new Date(Date.now() + 12 * 60_000).toISOString();
    expect(formatResetTime(soon)).toMatch(/^in \d+m$/);
  });
});
