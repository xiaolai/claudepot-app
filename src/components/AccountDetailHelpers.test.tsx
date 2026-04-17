import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { formatResetTime } from "./AccountDetailHelpers";

/**
 * formatResetTime is date-aware to fix the 7-day-window ambiguity
 * (e.g. "resets 14:30" when the actual reset is three days out). Lock
 * down the granularity tiers here so a future locale / format change
 * doesn't silently regress back to time-only output.
 */
describe("formatResetTime", () => {
  const NOW = new Date("2026-04-17T10:00:00"); // Friday 10:00 local

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  // Build an ISO string for a local-time instant offset from NOW.
  function localIsoOffset(opts: { addHours?: number; addDays?: number }): string {
    const d = new Date(NOW);
    if (opts.addHours) d.setHours(d.getHours() + opts.addHours);
    if (opts.addDays) d.setDate(d.getDate() + opts.addDays);
    return d.toISOString();
  }

  it("returns 'due' when reset is in the past", () => {
    expect(formatResetTime(localIsoOffset({ addHours: -1 }))).toBe("due");
  });

  it("returns 'in Nm' when reset is less than an hour away", () => {
    const in43m = new Date(NOW);
    in43m.setMinutes(in43m.getMinutes() + 43);
    expect(formatResetTime(in43m.toISOString())).toBe("in 43m");
  });

  it("returns 24-hour time-only when reset is later today", () => {
    // 14:30 same day — the common 5h window case. Locale is forced to
    // en-US with hour12:false, so this is a stable exact string.
    const sameDay = new Date(NOW);
    sameDay.setHours(14, 30, 0, 0);
    expect(formatResetTime(sameDay.toISOString())).toBe("14:30");
  });

  it("includes the short weekday when reset is within 7 days", () => {
    // +3 days from Friday 2026-04-17 → Monday.
    expect(formatResetTime(localIsoOffset({ addDays: 3 }))).toBe("Mon 10:00");
  });

  it("includes month+day+time when reset is 7 or more days away", () => {
    // +9 days from 2026-04-17 → 2026-04-26.
    expect(formatResetTime(localIsoOffset({ addDays: 9 }))).toBe(
      "Apr 26, 10:00",
    );
  });
});
