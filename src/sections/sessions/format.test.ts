import { describe, expect, it } from "vitest";
import { deriveSessionTitle } from "./format";

describe("deriveSessionTitle", () => {
  it("returns null for null, empty, and whitespace-only input", () => {
    expect(deriveSessionTitle(null)).toBeNull();
    expect(deriveSessionTitle("")).toBeNull();
    expect(deriveSessionTitle("   \n  \t ")).toBeNull();
  });

  it("passes a plain prompt through unchanged", () => {
    expect(deriveSessionTitle("hello world")).toBe("hello world");
  });

  it("strips CC's leading [Image #N] placeholder", () => {
    expect(deriveSessionTitle("[Image #3] can you see this?")).toBe(
      "can you see this?",
    );
  });

  it("strips multiple interleaved placeholders", () => {
    expect(
      deriveSessionTitle(
        "[Image #1] compare with [Image #2] and then [Pasted text #4 +12 lines] please",
      ),
    ).toBe("compare with  and then  please".replace(/\s+/g, " "));
  });

  it("strips [Pasted text #N] with and without the line count", () => {
    expect(deriveSessionTitle("[Pasted text #1] tail")).toBe("tail");
    expect(deriveSessionTitle("[Pasted text #9 +42 lines] tail")).toBe("tail");
  });

  it("strips [...Truncated text #N] placeholders", () => {
    expect(deriveSessionTitle("[...Truncated text #7] tail")).toBe("tail");
  });

  it("returns null if only placeholders remain", () => {
    expect(deriveSessionTitle("[Image #1]")).toBeNull();
    expect(
      deriveSessionTitle("  [Image #1] [Pasted text #2 +3 lines]  "),
    ).toBeNull();
  });

  it("strips leading Markdown header markers", () => {
    expect(deriveSessionTitle("# Title")).toBe("Title");
    expect(deriveSessionTitle("## Title")).toBe("Title");
    expect(deriveSessionTitle("###### Title")).toBe("Title");
  });

  it("strips leading code fences with optional language tag", () => {
    expect(deriveSessionTitle("```\nactual prompt")).toBe("actual prompt");
    expect(deriveSessionTitle("```ts\nactual prompt")).toBe("actual prompt");
    expect(deriveSessionTitle("```py-3\nactual prompt")).toBe("actual prompt");
  });

  it("strips leading blockquote markers", () => {
    expect(deriveSessionTitle("> quoted text")).toBe("quoted text");
    expect(deriveSessionTitle("> > nested quoted")).toBe("nested quoted");
  });

  it("peels stacked scaffolding in any order", () => {
    expect(deriveSessionTitle("> ## Stacked")).toBe("Stacked");
    expect(deriveSessionTitle("```md\n## Stacked")).toBe("Stacked");
  });

  it("collapses internal whitespace and preserves mid-sentence #", () => {
    expect(deriveSessionTitle("foo\n\nbar\tbaz")).toBe("foo bar baz");
    expect(deriveSessionTitle("bug # 123 reproduces")).toBe(
      "bug # 123 reproduces",
    );
  });

  it("does not strip placeholders embedded mid-sentence differently from leading ones", () => {
    // Both leading and mid-sentence placeholders are removed; the rest
    // of the sentence stays intact.
    expect(
      deriveSessionTitle("see [Image #1] for the screenshot"),
    ).toBe("see  for the screenshot".replace(/\s+/g, " "));
  });
});
