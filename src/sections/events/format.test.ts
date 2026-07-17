import { describe, expect, it } from "vitest";
import { displayPath, formatCompact } from "./format";

describe("formatCompact", () => {
  it("passes sub-1k counts through", () => {
    expect(formatCompact(0)).toBe("0");
    expect(formatCompact(999)).toBe("999");
  });

  it("renders k with one decimal", () => {
    expect(formatCompact(1_000)).toBe("1.0k");
    expect(formatCompact(12_345)).toBe("12.3k");
  });

  it("renders M and B with two decimals", () => {
    expect(formatCompact(4_560_000)).toBe("4.56M");
    expect(formatCompact(1_200_000_000)).toBe("1.20B");
  });
});

describe("displayPath", () => {
  it("returns the leaf folder of a Unix path", () => {
    expect(displayPath("/Users/joker/github/claudepot-app")).toBe(
      "claudepot-app",
    );
  });

  it("returns the leaf folder of a Windows path", () => {
    expect(displayPath("C:\\Users\\joker\\claudepot-app")).toBe(
      "claudepot-app",
    );
  });

  it("trims trailing separators", () => {
    expect(displayPath("/a/b/")).toBe("b");
    expect(displayPath("C:\\a\\b\\")).toBe("b");
  });

  it("passes empty input through", () => {
    expect(displayPath("")).toBe("");
  });
});
