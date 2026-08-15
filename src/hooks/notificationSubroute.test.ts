import { describe, expect, it } from "vitest";

/**
 * A notification that names a destination must reach that destination.
 *
 * `NotificationTarget` has carried `{ section: "settings", sub }` since
 * it was written, and the click router matched on the section and threw
 * the `sub` away — so the notification opened Settings on whichever
 * pane the user last had.
 *
 * The one producer is `src-tauri/src/retention_boot_check.rs`, which
 * dispatches `sub: "retention"`. That is the warning that Claude Code's
 * `cleanupPeriodDays` is deleting transcript history — per AGENTS.md the
 * only CC setting that destroys user data. The highest-stakes
 * notification in the app pointed at a pane and did not open it.
 *
 * Read as source: the router lives inside a `useEffect` closure with an
 * `api` dependency and a window-event round-trip, so mounting it to
 * observe one call would test the mock. What must not silently change
 * is that the subroute is forwarded at all.
 */
describe("notification click routing", () => {
  const SRC = import.meta.glob("./useNotificationClickRouter.ts", {
    eager: true,
    query: "?raw",
    import: "default",
  }) as Record<string, string>;
  const src = Object.values(SRC)[0] ?? "";

  it("loaded the router — otherwise this suite asserts nothing", () => {
    expect(src.length).toBeGreaterThan(500);
  });

  it("forwards a settings subroute instead of dropping it", () => {
    const at = src.indexOf('r.section === "settings"');
    expect(at, "the settings branch is gone").toBeGreaterThan(-1);
    const branch = src.slice(at, at + 700);
    expect(branch).toContain("r.sub");
    expect(branch).toContain("triggerSettingsTab");
  });

  it("does not lump settings in with a section that has no subroute", () => {
    // `r.section === "settings" || r.section === "events"` is how the
    // subroute got lost: one branch for two shapes.
    expect(src).not.toContain('r.section === "settings" || r.section === "events"');
  });
});
