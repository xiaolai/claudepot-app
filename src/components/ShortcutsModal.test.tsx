import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { ShortcutsModal } from "./ShortcutsModal";
import { sections } from "../sections/registry";

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

  it("lists one row per section, in registry order, with its ⌘ number", () => {
    const nav = renderNavGroup();
    const rows = within(nav).getAllByRole("listitem");
    // One row per section, plus the trailing ⌘, alias for Settings.
    expect(rows).toHaveLength(sections.length + 1);

    sections.forEach((section, i) => {
      const row = rows[i]!;
      expect(row.textContent).toContain(section.label);
      expect(row.textContent).toContain(String(i + 1));
    });
  });

  it("names no section the registry does not have", () => {
    const nav = renderNavGroup();
    const known = sections.map((s) => s.label);
    const rows = within(nav).getAllByRole("listitem");
    for (const row of rows.slice(0, sections.length)) {
      const matched = known.some((k) => row.textContent?.includes(k));
      expect(matched, `unknown section in row "${row.textContent}"`).toBe(true);
    }
  });

  it("still documents ⌘, as the standard Settings shortcut", () => {
    const nav = renderNavGroup();
    const rows = within(nav).getAllByRole("listitem");
    expect(rows[rows.length - 1]!.textContent).toMatch(/Settings/);
  });
});
