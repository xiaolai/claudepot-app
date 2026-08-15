import { sectionIds, sections } from "../sections/registry";

/**
 * The one table of keyboard bindings.
 *
 * `ShortcutsModal` used to hand-write everything except ⌘1..⌘9, and
 * the hand-written half drifted: it documented ⌘3 as "Sessions" and
 * ⌘4 as "Config" when neither was a section, put Settings on ⌘6 when
 * it was ⌘9, and never mentioned ⌘7..⌘9 at all. Deriving the section
 * numbers from the registry fixed that half. This finishes the job for
 * the other half — the fixed bindings — which had the inverse defect:
 *
 * - **⌘F** was documented for years while no section ever wired it. A
 *   shortcut that does nothing is worse than an undocumented one.
 * - **⌘\** is the mirror image: bound in `useSidebarCollapsed` since it
 *   was added, surfaced only in the sidebar toggle's tooltip, and
 *   absent from both this modal and `design.md`. A user finds it by
 *   hovering, or never.
 *
 * Both failures come from documentation and implementation being two
 * lists. This is the one list. A binding declared here without a
 * handler, or a handler without an entry here, is the thing to catch —
 * see `cargo xtask verify-docs`, which asserts every `key` named here
 * appears in a hook under `src/hooks/`.
 */
export interface ShortcutSpec {
  /** Rendered as <Kbd> chips, in order. */
  keys: string[];
  /** i18n key in the `components` namespace, under `shortcuts.`. */
  labelKey: string;
  /** Section label this binding is scoped to, if not global. */
  scopeSectionId?: string;
  /**
   * The literal `e.key` the handler compares against. Used by the
   * build gate to prove a handler exists; not rendered.
   */
  key: string;
}

/** Bindings that are always present, whatever the section list is. */
export const GLOBAL_SHORTCUTS: readonly ShortcutSpec[] = [
  { keys: ["⌘", "K"], labelKey: "openPalette", key: "k" },
  { keys: ["⌘", "/"], labelKey: "showShortcuts", key: "/" },
  { keys: ["⌘", "R"], labelKey: "refreshSection", key: "r" },
  {
    keys: ["⌘", "N"],
    labelKey: "addAccount",
    key: "n",
    scopeSectionId: "accounts",
  },
  {
    keys: ["⌘", "⇧", "C"],
    labelKey: "copyEmail",
    key: "c",
    scopeSectionId: "accounts",
  },
  { keys: ["⌘", "⇧", "L"], labelKey: "focusLive", key: "l" },
  // Was reachable only via the sidebar toggle's tooltip.
  { keys: ["⌘", "\\"], labelKey: "toggleSidebar", key: "\\" },
  // ⌘F focuses ConfigSection's content search. design.md asserted for
  // a long time that "There is no ⌘F" — true when written, and stale
  // since a section wired one. Undocumented working shortcuts are the
  // same defect as documented dead ones, just harder to notice.
  { keys: ["⌘", "F"], labelKey: "focusFilter", key: "f", scopeSectionId: "config" },
  { keys: ["⌘", ","], labelKey: "settingsStandard", key: "," },
  { keys: ["⌃", "⌥", "⌘", "B"], labelKey: "toggleBoards", key: "b" },
];

/**
 * ⌘-number for a section id, or `null` when it has none.
 *
 * Position in the **full registry**, matching `useSection`. Deriving
 * this from the *enabled* list is what made Settings' number depend on
 * whether Boards was switched on; both sides now read the same
 * ordering, so the modal cannot document a number the hook does not
 * bind.
 *
 * Sections past the ninth have no number — Settings is tenth and keeps
 * ⌘, instead.
 */
export function sectionNumber(id: string): number | null {
  const i = sectionIds.indexOf(id);
  if (i < 0 || i > 8) return null;
  return i + 1;
}

/** Every section that has a number, in registry order. */
export function numberedSections(): { id: string; n: number }[] {
  return sections
    .map((s) => ({ id: s.id, n: sectionNumber(s.id) }))
    .filter((x): x is { id: string; n: number } => x.n !== null);
}
