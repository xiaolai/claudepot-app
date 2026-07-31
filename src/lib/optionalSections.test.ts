// The filter that decides which sections exist.
//
// The failure this module guards against is asymmetric: a section that
// is VISIBLE but not navigable is a dead link, while one that is
// HIDDEN but still navigable is a surface the user believes they turned
// off. The second is worse, so every test here checks the hidden
// direction hardest.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  enabledSectionIds,
  enabledSections,
  isSectionEnabled,
  setSectionEnabled,
  toggleSection,
} from "./optionalSections";

const KEY = "claudepot.optional.boards";

beforeEach(() => localStorage.clear());
afterEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
});

describe("optionalSections", () => {
  it("treats an absent preference as off", () => {
    // A fresh install must not ship an on-trial section enabled.
    expect(localStorage.getItem(KEY)).toBeNull();
    expect(isSectionEnabled("boards")).toBe(false);
    expect(enabledSectionIds()).not.toContain("boards");
  });

  it("treats a corrupt value as off, not as on", () => {
    // Anything but "1" is off, so a truncated or hand-edited profile
    // fails closed rather than surfacing a section the user never
    // enabled.
    for (const junk of ["", "0", "true", "yes", "{}", "1 ", "01"]) {
      localStorage.setItem(KEY, junk);
      expect(isSectionEnabled("boards"), `value ${JSON.stringify(junk)}`).toBe(
        false,
      );
    }
  });

  it("round-trips an explicit enable", () => {
    expect(setSectionEnabled("boards", true)).toBe(true);
    expect(isSectionEnabled("boards")).toBe(true);
    expect(enabledSectionIds()).toContain("boards");
  });

  it("renumbers nothing before Boards, and only unnumbers Settings after it", () => {
    const before = enabledSectionIds();
    setSectionEnabled("boards", true);
    const after = enabledSectionIds();

    const boardsAt = after.indexOf("boards");
    expect(boardsAt).toBe(8); // ninth, so it takes ⌘9

    // ⌘1..⌘8 are stable across the toggle — that is the property that
    // makes this switchable without breaking muscle memory.
    before.slice(0, boardsAt).forEach((id, i) => expect(after[i]).toBe(id));

    // The one real consequence, stated rather than glossed: Settings
    // shifts from ninth to tenth, so it loses its ⌘9 binding while
    // Boards is on. That is the accepted trade — Settings keeps ⌘,
    // (see useShellShortcuts), which is its canonical shortcut.
    expect(before[8]).toBe("settings");
    expect(after[9]).toBe("settings");
  });

  it("reports the ACTUAL state when the write fails, not the requested one", () => {
    // The bug this locks: `setSectionEnabled` used to dispatch and
    // return the *requested* value. A caller then navigated to a
    // section that `enabledSections()` still excluded — invisible and
    // navigable at the same time.
    const spy = vi
      .spyOn(Storage.prototype, "setItem")
      .mockImplementation(() => {
        throw new Error("QuotaExceededError");
      });

    const actual = setSectionEnabled("boards", true);
    spy.mockRestore();

    // The in-memory fallback keeps the session self-consistent: what
    // the caller was told and what a reader sees must agree.
    expect(isSectionEnabled("boards")).toBe(actual);
    expect(enabledSectionIds().includes("boards")).toBe(actual);
  });

  it("stays self-consistent when storage cannot be read either", () => {
    const setSpy = vi
      .spyOn(Storage.prototype, "setItem")
      .mockImplementation(() => {
        throw new Error("no storage");
      });
    const getSpy = vi
      .spyOn(Storage.prototype, "getItem")
      .mockImplementation(() => {
        throw new Error("no storage");
      });

    const actual = setSectionEnabled("boards", true);
    expect(isSectionEnabled("boards")).toBe(actual);

    setSpy.mockRestore();
    getSpy.mockRestore();
  });

  it("toggle flips and returns the new state", () => {
    expect(toggleSection("boards")).toBe(true);
    expect(toggleSection("boards")).toBe(false);
    expect(isSectionEnabled("boards")).toBe(false);
  });

  it("announces the change so subscribers re-read", () => {
    const seen: unknown[] = [];
    const onChange = (e: Event) => seen.push((e as CustomEvent).detail);
    window.addEventListener("claudepot:optional-sections", onChange);
    setSectionEnabled("boards", true);
    window.removeEventListener("claudepot:optional-sections", onChange);
    expect(seen).toEqual([{ key: "boards", on: true }]);
  });

  it("never hides a non-optional section", () => {
    // Only sections that opted in can disappear. A bug here would take
    // Accounts or Settings out of the navigation.
    const all = enabledSections();
    expect(all.some((s) => s.id === "accounts")).toBe(true);
    expect(all.some((s) => s.id === "settings")).toBe(true);
  });
});
