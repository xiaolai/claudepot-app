import { describe, expect, it } from "vitest";
import type { Destination } from "./destination";
import {
  LEGACY_SECTION_IDS,
  canonicalSectionId,
  decodeDestination,
  encodeDestination,
  sameDestination,
} from "./destination";
import { sectionIds } from "../sections/registry";

describe("encodeDestination", () => {
  it("joins the levels it has and omits the ones it does not", () => {
    expect(encodeDestination({ section: "settings" })).toBe("settings");
    expect(encodeDestination({ section: "settings", tab: "retention" })).toBe(
      "settings/retention",
    );
    expect(
      encodeDestination({ section: "projects", tab: "config", sub: "repair" }),
    ).toBe("projects/config/repair");
  });

  /**
   * `{ section, sub }` with no `tab` is a TYPE error, not a runtime
   * one. It would encode to `section/sub` and decode back as a tab —
   * the address moving up a level and landing somewhere real, which is
   * the worst failure shape for a deep link. This asserts the escape
   * hatch stays closed.
   */
  it("cannot express a sub without a tab", () => {
    // @ts-expect-error — sub requires tab
    const illegal: Destination = { section: "projects", sub: "repair" };
    expect(illegal).toBeTruthy();
  });
});

describe("decodeDestination", () => {
  it("round-trips the canonical form", () => {
    const d = { section: "projects", tab: "config", sub: "repair" };
    expect(decodeDestination(encodeDestination(d))).toEqual(d);
  });

  it("accepts the colon form the app menu and tray emit", () => {
    // Those serialise through an OS API that carries only a string.
    expect(decodeDestination("settings:retention")).toEqual({
      section: "settings",
      tab: "retention",
    });
  });

  it("returns null for an unknown section rather than guessing", () => {
    // A wrong section is worse than none: every caller has a fallback,
    // and silently landing somewhere plausible is how a deep link
    // becomes untraceable.
    expect(decodeDestination("not-a-section")).toBeNull();
    expect(decodeDestination("")).toBeNull();
    expect(decodeDestination("   ")).toBeNull();
  });

  it("tolerates stray separators and whitespace", () => {
    expect(decodeDestination(" settings / retention ")).toEqual({
      section: "settings",
      tab: "retention",
    });
    expect(decodeDestination("settings//retention")).toEqual({
      section: "settings",
      tab: "retention",
    });
  });

  /**
   * These four ids are already written into users' localStorage. A
   * decode that rejects one drops that user back to Accounts, so they
   * are compatibility contracts rather than aliases we may retire.
   */
  it("accepts every legacy section id", () => {
    for (const legacy of Object.keys(LEGACY_SECTION_IDS)) {
      const d = decodeDestination(legacy);
      expect(d, `legacy id ${legacy} no longer decodes`).not.toBeNull();
      expect(sectionIds).toContain(d!.section);
    }
  });

  it("every registry section decodes to itself", () => {
    for (const id of sectionIds) {
      expect(decodeDestination(id)).toEqual({ section: id });
    }
  });
});

describe("sameDestination", () => {
  it("compares the path and ignores the entity", () => {
    const a = { section: "projects", tab: "sessions" };
    expect(sameDestination(a, { ...a })).toBe(true);
    expect(
      sameDestination(a, { ...a, target: { kind: "project" as const, id: "/x" } }),
    ).toBe(true);
    expect(sameDestination(a, { section: "projects" })).toBe(false);
  });
});

/**
 * The property A3 and A1 depend on.
 *
 * Restore paths read a raw string from `localStorage` and check it
 * against the current registry, falling back to Accounts when it does
 * not match. For a stale id from an older build that is right. For a
 * RENAMED one it silently discards the user's saved position, and the
 * two are indistinguishable at the call site — which is why the rename
 * map has to exist before the renames, not after.
 */
describe("canonicalSectionId", () => {
  it("passes through an id the registry already knows", () => {
    for (const id of sectionIds) {
      expect(canonicalSectionId(id)).toBe(id);
    }
  });

  it("resolves every legacy id to a section that exists", () => {
    for (const legacy of Object.keys(LEGACY_SECTION_IDS)) {
      const id = canonicalSectionId(legacy);
      expect(sectionIds, `${legacy} resolves to a section that is gone`).toContain(id);
    }
  });

  it("returns null for absent storage rather than a bare string", () => {
    expect(canonicalSectionId(null)).toBeNull();
    expect(canonicalSectionId(undefined)).toBeNull();
    expect(canonicalSectionId("")).toBeNull();
  });

  it("leaves an unknown id alone so the caller's own fallback runs", () => {
    // Not the map's job to invent a destination; the restore path
    // already validates against the registry.
    expect(canonicalSectionId("from-an-older-build")).toBe("from-an-older-build");
  });
});
