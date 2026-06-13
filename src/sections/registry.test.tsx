import { describe, expect, it } from "vitest";
import { sections, sectionIds } from "./registry";

describe("section registry — single source of truth", () => {
  it("keeps the legacy-locked ids, labels, and order", () => {
    // Ids are localStorage compat contracts ("events" → Activities,
    // "automations" → Agents, "third-party" → Providers). Order
    // drives the sidebar and ⌘1..⌘9. A failing assertion here is a
    // compat break, not a test to update.
    expect(sections.map((s) => [s.id, s.label])).toEqual([
      ["accounts", "Accounts"],
      ["events", "Activities"],
      ["projects", "Projects"],
      ["shared-memory", "Memory"],
      ["keys", "Keys"],
      ["third-party", "Providers"],
      ["automations", "Agents"],
      ["global", "Global"],
      ["settings", "Settings"],
    ]);
    expect(sectionIds).toEqual(sections.map((s) => s.id));
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
