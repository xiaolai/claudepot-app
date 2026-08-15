import { describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, within } from "@testing-library/react";
import { ShortcutsModal } from "./ShortcutsModal";
import {
  enabledSections,
  setSectionEnabled,
} from "../lib/optionalSections";
import { i18n } from "../lib/i18n";
import { sectionNumber } from "../lib/shortcutBindings";

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

  // ⌘1..⌘9 binds to position in the FULL registry, not in the visible
  // list, so a section's number is a property of that section. Only
  // the first nine registry entries have one at all.
  const BINDABLE = 9;

  it("gives each section the number its registry position implies", () => {
    const nav = renderNavGroup();
    const rows = within(nav).getAllByRole("listitem");
    // Assert on the NUMBERED rows, not the total. The list also carries
    // non-numbered aliases (⌘, and ⌃⌥⌘B), and a raw length check made
    // adding one of those look like a section-count regression.
    const numbered = rows.filter((r) => /[1-9]/.test(r.textContent ?? ""));

    // Visible sections that fall inside the first nine REGISTRY slots.
    const expected = enabledSections()
      .map((s) => ({ s, n: sectionNumber(s.id) }))
      .filter((x) => x.n !== null);
    expect(numbered).toHaveLength(expected.length);

    expected.forEach(({ s, n }, i) => {
      const row = numbered[i]!;
      expect(row.textContent).toContain(enShellT(s.labelKey));
      expect(row.textContent).toContain(String(n));
    });
  });

  /**
   * The regression this change exists for. Boards ships off and sits
   * ninth, so under position-in-the-VISIBLE-list numbering, enabling
   * it moved Settings off ⌘9 and handed the key to Boards — the exact
   * muscle-memory break the registry comment claimed Boards' ninth
   * position avoided, just conditional on a setting.
   *
   * Asserted on every section enabled in BOTH states, including
   * whether it has a number at all. An earlier version of this test
   * compared only sections appearing in both *numbered lists*, which
   * silently skipped the single section that regressed: under the bug
   * Settings is pushed to tenth and drops out of the list entirely, so
   * "present in both" excluded exactly the evidence.
   */
  it("does not renumber any section when an optional one is toggled", () => {
    setSectionEnabled("boards", false);
    const off = new Map(
      enabledSections().map((s) => [s.id, sectionNumber(s.id)]),
    );
    setSectionEnabled("boards", true);
    const on = new Map(
      enabledSections().map((s) => [s.id, sectionNumber(s.id)]),
    );

    let compared = 0;
    for (const [id, n] of off) {
      if (!on.has(id)) continue; // only Boards itself changes visibility
      compared += 1;
      expect(on.get(id), `${id} was renumbered by toggling Boards`).toBe(n);
    }
    expect(compared).toBeGreaterThan(3);
    // Settings must be in that comparison — it is the section the bug
    // moved, so a test that silently dropped it would prove nothing.
    expect(off.has("settings") && on.has("settings")).toBe(true);
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
