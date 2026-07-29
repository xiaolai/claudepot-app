import { describe, expect, it } from "vitest";
import { SETTINGS_PANES, isSettingsPaneId } from "./panes";
import { GLOBAL_TABS, isGlobalTabId } from "../global/tabs";
import { sections, sectionIds } from "../registry";

/**
 * These tables feed two consumers each: the section that renders them
 * and the ⌘K palette that deep-links into them. A pane the palette
 * can name but the section can't select is a dead command, so the
 * shapes are pinned here.
 */
describe("SETTINGS_PANES", () => {
  it("has unique ids", () => {
    const ids = SETTINGS_PANES.map((p) => p.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("recognizes its own ids and rejects others", () => {
    for (const p of SETTINGS_PANES) expect(isSettingsPaneId(p.id)).toBe(true);
    expect(isSettingsPaneId("sessions")).toBe(false);
    expect(isSettingsPaneId("")).toBe(false);
  });

  it("groups every pane as core or advanced", () => {
    for (const p of SETTINGS_PANES) {
      expect(["core", "advanced"]).toContain(p.group);
    }
  });
});

describe("GLOBAL_TABS", () => {
  it("has unique ids and a config default", () => {
    const ids = GLOBAL_TABS.map((t) => t.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toContain("config");
  });

  it("recognizes its own ids and rejects others", () => {
    for (const t of GLOBAL_TABS) expect(isGlobalTabId(t.id)).toBe(true);
    expect(isGlobalTabId("nope")).toBe(false);
  });
});

/**
 * The Settings "Open on launch" picker is built from the registry.
 * It used to be a hand-written list that offered "Sessions" — an id
 * that no longer existed, so `useSection` rejected it and silently
 * landed the user on Accounts — while omitting four real sections.
 */
describe("launch-section options", () => {
  it("offers exactly the sections useSection will accept", async () => {
    const mod = await import("../registry");
    for (const s of mod.sections) {
      expect(sectionIds).toContain(s.id);
    }
    expect(sectionIds).toHaveLength(sections.length);
  });

  it("has no id that the registry does not define", () => {
    // The specific ghost that shipped: a "sessions" section.
    expect(sectionIds).not.toContain("sessions");
  });
});
