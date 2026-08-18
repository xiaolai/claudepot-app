import { describe, expect, it } from "vitest";
import { formatElapsed } from "./elapsed";

const S = 1000;
const M = 60 * S;
const H = 60 * M;

describe("formatElapsed", () => {
  it("shows seconds under two minutes, where they are still information", () => {
    expect(formatElapsed(0)).toBe("0s");
    expect(formatElapsed(42 * S)).toBe("42s");
    expect(formatElapsed(M + 42 * S)).toBe("1m 42s");
  });

  // The whole reason the formatter is adaptive: a 15-minute run ticking
  // seconds is noise, and a flickering ambient row gets ignored.
  it("drops seconds from two minutes onward", () => {
    expect(formatElapsed(2 * M)).toBe("2m");
    expect(formatElapsed(14 * M + 46 * S)).toBe("14m");
    expect(formatElapsed(59 * M + 59 * S)).toBe("59m");
  });

  it("switches to hours at the hour boundary", () => {
    expect(formatElapsed(H)).toBe("1h 0m");
    expect(formatElapsed(H + 12 * M)).toBe("1h 12m");
    expect(formatElapsed(25 * H + 3 * M)).toBe("25h 3m");
  });

  // Clock skew between the backend's started_ms and the renderer's
  // Date.now() must not render "-3s", which reads as an app bug.
  it("clamps a negative duration to zero rather than rendering it", () => {
    expect(formatElapsed(-1)).toBe("0s");
    expect(formatElapsed(-60 * M)).toBe("0s");
  });

  // Exact boundaries, because off-by-one here is invisible in review.
  it("is exact at every band boundary", () => {
    expect(formatElapsed(2 * M - 1)).toBe("1m 59s");
    expect(formatElapsed(H - 1)).toBe("59m");
  });
});
