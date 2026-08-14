/**
 * Unit tests for the *judgement* half of `check-envvar-layout.mjs`.
 *
 * The guard itself cannot run in CI — it drives the real app over the
 * debug-only MCP bridge, which needs a macOS GUI session, a Vite
 * server and a windowed debug build. `AGENTS.md` names this script as
 * the example of a check nobody has watched fail, which is
 * indistinguishable from one that cannot fail.
 *
 * Splitting `evaluate()` out from the measurement fixes the half that
 * is fixable. Feeding it a synthetic 0px measurement proves the
 * assertions still bite without needing a screen; `--self-test` does
 * the same thing against the live pane, for the half that genuinely
 * needs one.
 *
 * The fixtures below are the shapes `MEASURE` returns, and the 0px one
 * is the bug that prompted the whole script: `.envvar-list` resolving
 * to `flex-basis: 0` with every row in the DOM and none on screen.
 */
import { describe, expect, it } from "vitest";
import { evaluate } from "./check-envvar-layout.mjs";

const healthy = {
  listHeight: 420,
  listScrollHeight: 900,
  visibleRows: 9,
  totalRows: 60,
  scrollers: ["envvar-list"],
  bucketsInsideList: true,
  bucketCount: 3,
};
/** The shipped bug: list pinned to basis 0, nothing visible. */
const zeroPx = { ...healthy, listHeight: 0, visibleRows: 0 };

describe("check-envvar-layout — evaluate()", () => {
  it("passes a healthy pane", () => {
    expect(evaluate(healthy, healthy)).toEqual([]);
  });

  it("flags a zero-height list and says how much is clipped", () => {
    const f = evaluate(zeroPx, zeroPx).join("\n");
    expect(f).toContain("height 0px");
    expect(f).toContain("900px of content is clipped");
  });

  it("flags that no row intersects the viewport", () => {
    expect(evaluate(zeroPx, zeroPx).join("\n")).toContain("0 of 60 rows");
  });

  it("reports both disclosure states, not just the default", () => {
    // The original bug scored 136px collapsed and 0px expanded, so a
    // guard that only measured the default would have understated it.
    const f = evaluate(zeroPx, zeroPx);
    expect(f.filter((m) => m.startsWith("[collapsed]"))).toHaveLength(2);
    expect(f.filter((m) => m.startsWith("[expanded]"))).toHaveLength(2);
  });

  it("catches a break that only appears when the appendix is expanded", () => {
    expect(evaluate(healthy, zeroPx).length).toBeGreaterThan(0);
  });

  it("flags nested scroll containers", () => {
    const nested = { ...healthy, scrollers: ["envvar-pane", "envvar-list"] };
    expect(evaluate(nested, nested).join("\n")).toContain("scroll container");
  });

  it("flags an appendix bucket rendered outside the list", () => {
    const outside = { ...healthy, bucketsInsideList: false };
    expect(evaluate(outside, outside).join("\n")).toContain(
      "outside .envvar-list",
    );
  });

  it("does not flag bucket placement when there are no buckets", () => {
    const none = { ...healthy, bucketCount: 0, bucketsInsideList: false };
    expect(evaluate(none, none)).toEqual([]);
  });
});
