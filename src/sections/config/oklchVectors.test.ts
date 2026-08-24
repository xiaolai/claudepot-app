import { describe, expect, it } from "vitest";
import vectors from "../../../crates/claudepot-core/testdata/oklch-vectors.json";
import { toMermaidColor } from "./mermaidColor";

/**
 * The desktop half of the shared vectors.
 *
 * `panel/src/app/color.js` runs the same file in
 * `panel/src/app/oklchVectors.test.js`. The two implementations cannot
 * import each other — the panel is a separate Vite app — so this is the
 * only thing that keeps them equal, and their doc comments both point
 * here. Same arrangement as `PriceBook::resolve` / `src/costs.ts` and
 * `session::title::derive` / `deriveSessionTitle`.
 */
describe("oklch shared vectors (desktop)", () => {
  it("has vectors to run", () => {
    expect(vectors.cases.length).toBeGreaterThan(10);
  });

  for (const { input, expected } of vectors.cases) {
    it(`converts ${JSON.stringify(input)}`, () => {
      expect(toMermaidColor(input)).toBe(expected);
    });
  }
});
