import { describe, expect, it } from "vitest";
import { hasLanguage } from "./codeFence";

/**
 * Twin of the `hasLanguage` cases in `panel/src/app/markdown.test.js`.
 * Both implementations must agree; change one, change the other.
 */
describe("hasLanguage", () => {
  it("matches the exact language token", () => {
    expect(hasLanguage("language-mermaid", "mermaid")).toBe(true);
    expect(hasLanguage("hljs language-mermaid extra", "mermaid")).toBe(true);
  });

  it("does not match a language that merely starts the same", () => {
    // The defect: `includes("language-mermaid")` routed these to the
    // diagram renderer, which then failed to parse ordinary code.
    expect(hasLanguage("language-mermaidish", "mermaid")).toBe(false);
    expect(hasLanguage("language-mermaid-extra", "mermaid")).toBe(false);
  });

  it("is false for other languages and for nothing at all", () => {
    expect(hasLanguage("language-rust", "mermaid")).toBe(false);
    expect(hasLanguage("", "mermaid")).toBe(false);
    expect(hasLanguage(undefined, "mermaid")).toBe(false);
  });
});
