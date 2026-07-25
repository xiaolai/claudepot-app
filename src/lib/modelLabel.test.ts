import { describe, expect, it } from "vitest";

import { compactModelLabel } from "./modelLabel";

describe("compactModelLabel", () => {
  it("renders the major-minor id shape with a dotted version", () => {
    expect(compactModelLabel("claude-opus-4-7")).toBe("Opus 4.7");
    expect(compactModelLabel("claude-sonnet-4-6")).toBe("Sonnet 4.6");
    expect(compactModelLabel("claude-haiku-4-5")).toBe("Haiku 4.5");
  });

  it("renders the dateless id shape introduced with the 5 generation", () => {
    // The regression this helper was extracted to fix: the Sessions
    // copy required three segments and fell through to the raw id here,
    // so the default Opus model rendered as `opus-5`.
    expect(compactModelLabel("claude-opus-5")).toBe("Opus 5");
    expect(compactModelLabel("claude-sonnet-5")).toBe("Sonnet 5");
    expect(compactModelLabel("claude-fable-5")).toBe("Fable 5");
  });

  it("drops snapshot date stamps", () => {
    expect(compactModelLabel("claude-haiku-4-5-20251001")).toBe("Haiku 4.5");
    expect(compactModelLabel("claude-opus-5-20260724")).toBe("Opus 5");
  });

  it("drops alias markers", () => {
    expect(compactModelLabel("claude-sonnet-4-6-preview")).toBe("Sonnet 4.6");
    expect(compactModelLabel("claude-opus-5-latest")).toBe("Opus 5");
    expect(compactModelLabel("claude-opus-5-experimental")).toBe("Opus 5");
  });

  it("drops a stacked snapshot + alias in either order", () => {
    // A single anchored strip of each only handles one order; with the
    // alias last, the date survived into the label.
    expect(compactModelLabel("claude-opus-5-20260724-preview")).toBe("Opus 5");
    expect(compactModelLabel("claude-haiku-4-5-20251001-latest")).toBe(
      "Haiku 4.5",
    );
  });

  it("returns non-Claude ids verbatim", () => {
    // Forcing these into the pattern would invent a label the user
    // can't match against anything they configured.
    expect(compactModelLabel("us.anthropic.claude-sonnet-4-5-v1:0")).toBe(
      "us.anthropic.claude-sonnet-4-5-v1:0",
    );
    expect(compactModelLabel("gpt-4")).toBe("gpt-4");
    expect(compactModelLabel("")).toBe("");
  });

  it("survives degenerate Claude-prefixed ids", () => {
    expect(compactModelLabel("claude-opus")).toBe("Opus");
    expect(compactModelLabel("claude-")).toBe("claude-");
  });

  it("trims surrounding whitespace", () => {
    expect(compactModelLabel("  claude-opus-5  ")).toBe("Opus 5");
  });
});
