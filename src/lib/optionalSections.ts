/**
 * Sections the user can switch off.
 *
 * # Why this exists as ONE module
 *
 * `AGENTS.md` is explicit that everything enumerating sections reads
 * the registry — the ⌘K palette, the ⌘1..⌘9 bindings, the shortcuts
 * reference, the sidebar, and Settings' launch picker. A *second* list
 * is a review finding, and the reason is on record: three of those four
 * had already drifted once.
 *
 * Hiding a section has exactly the same hazard. Filtering it out of the
 * sidebar alone would leave it reachable by ⌘9, by the palette, and as
 * a saved launch target — visibly absent but still navigable, which is
 * worse than either state. So the filter lives here, and every consumer
 * derives its list from [`enabledSections`].
 *
 * # Default off
 *
 * Boards ships disabled. It is on trial (see
 * `dev-docs/agent-boards-plan.md` §10.1) and the point of the trial is
 * to find out whether it earns a permanent place. A feature that
 * installs itself into the navigation before answering that question
 * has prejudged it.
 */

import { sections, type SectionDef } from "../sections/registry";
import { optionalSectionKey } from "./storageKeys";

/** Keys of sections that can be switched off. */
export type OptionalSectionKey = "boards";

/**
 * localStorage key per optional section. Value is `"1"` or `"0"`.
 *
 * Built from `storageKeys.ts` so the registry's preload guards — which
 * cannot import this module — read the same key rather than a
 * hand-copied literal.
 */
const STORAGE_KEY: Record<OptionalSectionKey, string> = {
  boards: optionalSectionKey("boards"),
};

/**
 * Fired when an optional section is toggled.
 *
 * A plain custom event rather than a context provider: the consumers
 * are spread across hooks, the shell, and a lazy Settings pane, and
 * threading a provider through all of them to carry one boolean would
 * be more coupling than the feature is worth.
 */
export const OPTIONAL_SECTIONS_EVENT = "claudepot:optional-sections";

export function isSectionEnabled(key: OptionalSectionKey): boolean {
  try {
    // Strict "1" match, so absent, "0", and any corrupt value all read
    // as off. A new install and a garbage value must behave identically.
    return localStorage.getItem(STORAGE_KEY[key]) === "1";
  } catch {
    // Storage unreadable — fall back to the in-memory mirror, which is
    // still off unless this session explicitly turned it on.
    return memory.get(key) ?? false;
  }
}

/**
 * In-memory mirror, used when localStorage is unavailable.
 *
 * Without it a failed write left the caller believing the section was
 * on while `isSectionEnabled` still read false — producing exactly the
 * invisible-but-navigable state this module exists to prevent. The
 * fallback keeps the session self-consistent; it just does not persist.
 */
const memory = new Map<OptionalSectionKey, boolean>();

/**
 * Set a section's state and return what it ACTUALLY became.
 *
 * Never reports the requested value: a caller that navigates on the
 * strength of a write that failed is the bug this signature prevents.
 */
export function setSectionEnabled(
  key: OptionalSectionKey,
  on: boolean,
): boolean {
  try {
    localStorage.setItem(STORAGE_KEY[key], on ? "1" : "0");
    memory.delete(key);
  } catch {
    // Private mode / quota. Hold it in memory so reads agree with
    // what the caller was told, for this session only.
    memory.set(key, on);
  }
  const actual = isSectionEnabled(key);
  window.dispatchEvent(
    new CustomEvent(OPTIONAL_SECTIONS_EVENT, { detail: { key, on: actual } }),
  );
  return actual;
}

/** Flip one optional section and return its new state. */
export function toggleSection(key: OptionalSectionKey): boolean {
  return setSectionEnabled(key, !isSectionEnabled(key));
}

/**
 * The sections that should appear anywhere in the UI right now.
 *
 * Order is preserved from the registry, so a disabled section does not
 * renumber the ⌘ bindings of the ones before it — Boards sits ninth
 * precisely so switching it on or off never moves anything else.
 */
export function enabledSections(): readonly SectionDef[] {
  return sections.filter(
    (s) => !s.optional || isSectionEnabled(s.optional),
  );
}

export function enabledSectionIds(): readonly string[] {
  return enabledSections().map((s) => s.id);
}
