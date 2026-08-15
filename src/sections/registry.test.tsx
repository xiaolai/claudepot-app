import { describe, expect, it } from "vitest";
import { i18n } from "../lib/i18n";
import { sections, sectionIds } from "./registry";

describe("section registry — single source of truth", () => {
  it("keeps the legacy-locked ids, label keys, and order", () => {
    // Ids are localStorage compat contracts ("events" → Activities,
    // "automations" → Agents, "third-party" → Providers). Order
    // drives the sidebar and ⌘1..⌘9. A failing assertion here is a
    // compat break, not a test to update. Display labels live in the
    // locale catalogs (src/locales/*/shell.json); the English values
    // are asserted below.
    expect(sections.map((s) => [s.id, s.labelKey])).toEqual([
      ["accounts", "sections.accounts"],
      ["events", "sections.events"],
      ["projects", "sections.projects"],
      ["shared-memory", "sections.shared-memory"],
      ["keys", "sections.keys"],
      ["third-party", "sections.third-party"],
      ["automations", "sections.automations"],
      ["config", "sections.config"],
      // Ninth on purpose so it takes ⌘9; see the registry comment.
      ["boards", "sections.boards"],
      ["settings", "sections.settings"],
    ]);
    expect(sectionIds).toEqual(sections.map((s) => s.id));
  });

  it("resolves every labelKey in both locales", () => {
    // en labels are the pre-i18n English contract; zh-CN must cover
    // every section so the localized sidebar never shows a raw key.
    const en = i18n.getFixedT("en", "shell");
    const zh = i18n.getFixedT("zh-CN", "shell");
    expect(sections.map((s) => en(s.labelKey))).toEqual([
      "Accounts",
      "Activities",
      "Projects",
      "Knowledge",
      "Keys",
      "Providers",
      "Agents",
      "Config",
      "Boards",
      "Settings",
    ]);
    for (const s of sections) {
      const v = zh(s.labelKey);
      expect(v, `zh-CN missing ${s.labelKey}`).not.toBe(s.labelKey);
      expect(v.length).toBeGreaterThan(0);
    }
  });

  it("every section except the eager accounts entry has a loader", () => {
    for (const s of sections) {
      if (s.id === "accounts") {
        expect(s.loader).toBeUndefined();
      } else {
        expect(s.loader, `section ${s.id} must code-split`).toBeTypeOf(
          "function",
        );
      }
    }
  });

  it("every section has a render function", () => {
    for (const s of sections) {
      expect(s.render, `section ${s.id}`).toBeTypeOf("function");
    }
  });
});
