import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { navigateTo, sectionsWithTabs } from "./navigateTo";
import { STORAGE_KEY_SETTINGS_TAB } from "./networkPanelDeepLink";
import { sectionIds } from "../sections/registry";

describe("navigateTo", () => {
  beforeEach(() => {
    try {
      sessionStorage.clear();
    } catch {
      /* jsdom always has it; guard matches production */
    }
  });
  afterEach(() => vi.restoreAllMocks());

  it("sets the section for a bare destination", () => {
    const setSection = vi.fn();
    navigateTo(setSection, { section: "accounts" });
    expect(setSection).toHaveBeenCalledWith("accounts", null);
  });

  /**
   * `setSection(id, sub)` takes both together so a deep link does not
   * flash the section's previous sub-route before correcting itself.
   */
  it("passes the sub-route in the same call as the section", () => {
    const setSection = vi.fn();
    navigateTo(setSection, { section: "projects", tab: "x", sub: "repair" });
    expect(setSection).toHaveBeenCalledWith("projects", "repair");
  });

  it("fires the tab trigger for a section that has one", () => {
    const setSection = vi.fn();
    navigateTo(setSection, { section: "settings", tab: "retention" });
    expect(setSection).toHaveBeenCalledWith("settings", null);
    // Both halves: the hint covers a cold mount, the event a hot one.
    expect(sessionStorage.getItem(STORAGE_KEY_SETTINGS_TAB)).toBe("retention");
  });

  it("ignores a tab on a section that has no tab level", () => {
    // A programming error, not a reason to guess: silently routing it
    // somewhere plausible is how a deep link becomes untraceable.
    const setSection = vi.fn();
    expect(() =>
      navigateTo(setSection, { section: "accounts", tab: "nonsense" }),
    ).not.toThrow();
    expect(setSection).toHaveBeenCalledWith("accounts", null);
  });

  it("every section with a tab trigger is a real section", () => {
    for (const id of sectionsWithTabs()) {
      expect(sectionIds, `${id} has a tab trigger but is not a section`).toContain(id);
    }
  });
});
