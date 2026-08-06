import { describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, within } from "@testing-library/react";
import { ShortcutsModal } from "./ShortcutsModal";
import {
  enabledSections,
  setSectionEnabled,
} from "../lib/optionalSections";
import { i18n } from "../lib/i18n";

// English section labels — tests run with the en locale active.
const enShellT = i18n.getFixedT("en", "shell");

/**
 * ⌘1..⌘9 is bound by *position* in the section registry (see
 * `useSection`), so this modal is only correct if it reads the same
 * list.
 *
 * The previous test asserted the literal strings "Sessions" and
 * "Config" were navigation rows. Both had stopped being top-level
 * sections; the modal claimed ⌘3 → Sessions, ⌘4 → Config and ⌘6 →
 * Settings while the app bound ⌘3 → Projects, ⌘4 → Knowledge and ⌘9 →
 * Settings. The test passed the whole time because it pinned the
 * documentation to itself rather than to the bindings. These tests
 * compare against the registry instead, so the same drift fails.
 */
describe("ShortcutsModal — navigation reflects the real bindings", () => {
  function renderNavGroup(): HTMLElement {
    render(<ShortcutsModal onClose={vi.fn()} />);
    // The <section> that wraps the "Navigation" heading and its list.
    return screen.getByText("Navigation").parentElement!;
  }

  // `useSection` binds ⌘1..⌘9 by position, so only the first NINE
  // sections have a number. This used to read `sections.length` because
  // there happened to be exactly nine; adding Boards as a tenth made
  // that coincidence load-bearing.
  const BINDABLE = 9;


  it("lists one row per BINDABLE section, in registry order, with its ⌘ number", () => {
    const nav = renderNavGroup();
    const rows = within(nav).getAllByRole("listitem");
    // Assert on the NUMBERED rows, not the total. The list also carries
    // non-numbered aliases (⌘, and ⌃⌥⌘B), and a raw length check made
    // adding one of those look like a section-count regression.
    const sections = enabledSections();
    const numbered = rows.filter((r) => /[1-9]/.test(r.textContent ?? ""));
    expect(numbered).toHaveLength(Math.min(sections.length, BINDABLE));

    sections.slice(0, BINDABLE).forEach((section, i) => {
      const row = rows[i]!;
      expect(row.textContent).toContain(enShellT(section.labelKey));
      expect(row.textContent).toContain(String(i + 1));
    });
  });

  it("does not claim a ⌘ number for a section past the ninth", () => {
    // Enable Boards so a tenth section actually exists — the previous
    // version bailed out via an early `return` whenever the registry
    // had nine or fewer, which is the default, so it never ran.
    setSectionEnabled("boards", true);
    const sections = enabledSections();
    expect(sections.length).toBeGreaterThan(BINDABLE);

    const nav = renderNavGroup();
    const numbered = within(nav)
      .getAllByRole("listitem")
      .map((r) => r.textContent ?? "")
      .filter((t) => /[1-9]/.test(t));

    for (const extra of sections.slice(BINDABLE)) {
      const label = enShellT(extra.labelKey);
      expect(
        numbered.some((t) => t.includes(label)),
        `${label} claims a ⌘ number it does not have`,
      ).toBe(false);
    }
    cleanup();
    setSectionEnabled("boards", false);
  });

  it("names no section the registry does not have", () => {
    const nav = renderNavGroup();
    const known = enabledSections().map((s) => enShellT(s.labelKey));
    const rows = within(nav).getAllByRole("listitem");
    for (const row of rows.slice(0, Math.min(enabledSections().length, BINDABLE))) {
      const matched = known.some((k) => row.textContent?.includes(k));
      expect(matched, `unknown section in row "${row.textContent}"`).toBe(true);
    }
  });

  it("still documents ⌘, as the standard Settings shortcut", () => {
    const nav = renderNavGroup();
    const text = within(nav)
      .getAllByRole("listitem")
      .map((r) => r.textContent ?? "");
    expect(text.some((t) => /Settings/.test(t) && t.includes(","))).toBe(true);
  });

  it("documents ⌃⌥⌘B, and lists a NUMBERED Boards row only once enabled", () => {
    // The first version of this test asserted `t.includes("Boards")`,
    // which the always-present "Show / hide Boards" shortcut row
    // satisfies — so it passed while the modal was in fact stale and
    // never showed the section at all. Assert the NUMBERED row.
    const numberedBoards = (rows: string[]) =>
      rows.some((t) => /Boards/.test(t) && /[1-9]/.test(t));

    setSectionEnabled("boards", false);
    let text = within(renderNavGroup())
      .getAllByRole("listitem")
      .map((r) => r.textContent ?? "");
    expect(text.some((t) => t.includes("Show / hide Boards"))).toBe(true);
    expect(numberedBoards(text)).toBe(false);
    cleanup();

    setSectionEnabled("boards", true);
    text = within(renderNavGroup())
      .getAllByRole("listitem")
      .map((r) => r.textContent ?? "");
    // Boards is the ninth section, so it must carry ⌘9.
    expect(text.some((t) => /Boards/.test(t) && t.includes("9"))).toBe(true);
    cleanup();
    setSectionEnabled("boards", false);
  });
});
