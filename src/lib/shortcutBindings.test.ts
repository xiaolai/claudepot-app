import { describe, expect, it } from "vitest";
import enComponents from "../locales/en/components.json";
import zhComponents from "../locales/zh-CN/components.json";
import { GLOBAL_SHORTCUTS, numberedSections, sectionNumber } from "./shortcutBindings";
import { sectionIds } from "../sections/registry";

/**
 * `ShortcutsModal` renders these bindings by building the i18n key at
 * runtime (`shortcuts.${labelKey}`), which i18next's typed `t()` cannot
 * check. That compile-time guarantee is replaced here: a labelKey with
 * no catalog entry fails this test instead of rendering its own key
 * back at the user.
 *
 * Both catalogs are asserted. `check:catalogs` already enforces en↔zh
 * parity, so a zh miss would surface there too — but only if the key
 * exists in en, and the failure this guards is the key existing in
 * NEITHER.
 */
describe("GLOBAL_SHORTCUTS", () => {
  it("is non-empty — an empty table would pass every assertion below", () => {
    expect(GLOBAL_SHORTCUTS.length).toBeGreaterThan(5);
  });

  it("every labelKey resolves in both catalogs", () => {
    const en = enComponents.shortcuts as Record<string, string>;
    const zh = zhComponents.shortcuts as Record<string, string>;
    for (const b of GLOBAL_SHORTCUTS) {
      expect(en[b.labelKey], `en is missing shortcuts.${b.labelKey}`).toBeTruthy();
      expect(zh[b.labelKey], `zh-CN is missing shortcuts.${b.labelKey}`).toBeTruthy();
    }
  });

  it("every scopeSectionId names a real section", () => {
    for (const b of GLOBAL_SHORTCUTS) {
      if (!b.scopeSectionId) continue;
      expect(sectionIds, `${b.labelKey} scopes to an unknown section`).toContain(
        b.scopeSectionId,
      );
    }
  });

  it("declares no duplicate key + modifier combination", () => {
    const seen = new Set<string>();
    for (const b of GLOBAL_SHORTCUTS) {
      const combo = `${[...b.keys].sort().join("+")}|${b.key}`;
      expect(seen.has(combo), `${combo} is declared twice`).toBe(false);
      seen.add(combo);
    }
  });
});

describe("sectionNumber", () => {
  it("numbers the first nine registry entries and no more", () => {
    const numbered = numberedSections();
    expect(numbered).toHaveLength(Math.min(9, sectionIds.length));
    numbered.forEach((s, i) => expect(s.n).toBe(i + 1));
  });

  it("returns null for a section past the ninth and for an unknown id", () => {
    if (sectionIds.length > 9) expect(sectionNumber(sectionIds[9])).toBeNull();
    expect(sectionNumber("not-a-section")).toBeNull();
  });

  /**
   * The property the ⌘-number change exists for: a section's number is
   * a function of the registry alone, so nothing about which sections
   * are currently visible can move it.
   */
  it("depends only on registry position", () => {
    for (const [i, id] of sectionIds.entries()) {
      expect(sectionNumber(id)).toBe(i < 9 ? i + 1 : null);
    }
  });
});
