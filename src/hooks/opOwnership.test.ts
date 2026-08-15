import { describe, expect, it } from "vitest";

/**
 * A modal owns its operation's ambient tier.
 *
 * `App` mounts both `RunningOpsChip` (in the status bar) and
 * `OperationProgressHost`. Before this, one rename rendered as "1 op"
 * in the bar *directly beneath* the modal already showing its phases —
 * two ambient surfaces for one event, which the signal budget in
 * `rules/design.md` forbids.
 *
 * Read as source rather than executed: the filter lives in `App`, whose
 * mount needs the Tauri bridge, every provider and a live section tree.
 * Rendering all of that to observe one `.filter` would test the mock.
 * What must not silently change is that the filter exists and keys on
 * the active op id.
 */
describe("running-ops chip vs. the progress modal", () => {
  const SRC = import.meta.glob("../App.tsx", {
    eager: true,
    query: "?raw",
    import: "default",
  }) as Record<string, string>;
  const src = Object.values(SRC)[0] ?? "";

  it("loaded App.tsx — otherwise this suite asserts nothing", () => {
    expect(src.length).toBeGreaterThan(1000);
  });

  it("filters the modal-owned op out of the chip's list", () => {
    const at = src.indexOf("const runningOps = useMemo(");
    expect(at, "the chip no longer derives its ops from a filter").toBeGreaterThan(-1);
    const body = src.slice(at, src.indexOf(");", at));
    expect(body).toContain("activeOp?.opId");
    expect(body).toContain("filter");
  });

  it("passes the FILTERED list to the status bar, not the raw one", () => {
    const at = src.indexOf("<AppStatusBar");
    expect(at).toBeGreaterThan(-1);
    const props = src.slice(at, src.indexOf("/>", at));
    expect(props).toContain("runningOps={runningOps}");
    expect(props).not.toContain("allRunningOps");
  });
});
