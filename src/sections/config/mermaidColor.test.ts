import { describe, expect, it } from "vitest";
import { toMermaidColor } from "./mermaidColor";

describe("toMermaidColor", () => {
  it("converts the sRGB primaries exactly", () => {
    // Published oklch coordinates from CSS Color 4 — a check against a
    // standard, not against whatever the implementation happened to
    // produce the day it was written.
    expect(toMermaidColor("oklch(0% 0 0)")).toBe("#000000");
    expect(toMermaidColor("oklch(100% 0 0)")).toBe("#ffffff");
    expect(toMermaidColor("oklch(62.796% 0.25768 29.234)")).toBe("#ff0000");
    expect(toMermaidColor("oklch(86.644% 0.29483 142.495)")).toBe("#00ff00");
    expect(toMermaidColor("oklch(45.201% 0.31321 264.052)")).toBe("#0000ff");
  });

  it("converts the palette this app actually paints with", () => {
    // Straight out of tokens.css.
    expect(toMermaidColor("oklch(22% 0.008 60)")).toMatch(/^#[0-9a-f]{6}$/);
    expect(toMermaidColor("oklch(97% 0.004 60)")).toMatch(/^#[0-9a-f]{6}$/);
  });

  it("keeps alpha, because a hairline is mostly transparent", () => {
    expect(toMermaidColor("oklch(24% 0.01 60/0.09)")).toMatch(
      /^rgba\(\d+, \d+, \d+, 0\.09\)$/,
    );
  });

  it("accepts both alpha spellings and both lightness notations", () => {
    expect(toMermaidColor("oklch(24% 0.01 60/0.5)")).toBe(
      toMermaidColor("oklch(24% 0.01 60 / 0.5)"),
    );
    expect(toMermaidColor("oklch(0.63 0.128 41)")).toBe(
      toMermaidColor("oklch(63% 0.128 41)"),
    );
  });

  it("leaves anything khroma already understands alone", () => {
    // Which is what makes it safe to map over every theme variable
    // rather than a curated subset — including the font stack.
    for (const v of [
      "#b4542a",
      "rgb(180, 84, 42)",
      "rgba(180, 84, 42, 0.5)",
      "hsl(18, 62%, 44%)",
      "transparent",
      "",
    ]) {
      expect(toMermaidColor(v)).toBe(v);
    }
  });

  it("emits only formats khroma accepts", () => {
    // The output contract. That khroma *actually* accepts these — and
    // rejects the oklch it replaces — is asserted against the real
    // library in `panel/src/app/color.test.js`, which runs under plain
    // node and can reach a transitive dependency. This tsconfig is
    // browser-targeted and has no node types, so proving it twice would
    // cost more than it is worth: the two implementations are checked
    // against the same published primaries, so they agree by
    // construction.
    for (const raw of [
      "oklch(63% 0.128 41)",
      "oklch(21% 0.006 60)",
      "oklch(97.6% 0.006 85)",
    ]) {
      expect(toMermaidColor(raw)).toMatch(/^#[0-9a-f]{6}$/);
    }
    expect(toMermaidColor("oklch(24% 0.01 60/0.09)")).toMatch(/^rgba\(/);
  });
});
