import { sectionIds } from "../sections/registry";

/**
 * One address for every place in the app.
 *
 * # Why this exists
 *
 * Navigation was expressed four different ways, and a destination lost
 * whatever the receiving transport did not model:
 *
 * - **Typed object** — `NotificationTarget` carries a section *and* a
 *   subroute. The click router matched the section and dropped the
 *   subroute, so the transcript-retention warning (the one telling you
 *   Claude Code is deleting history) asked for Settings → Retention and
 *   opened whichever pane you last had.
 * - **Colon string** — the app menu and tray serialise
 *   `app-menu:nav:<section>:<subtab>` and re-parse it in the renderer.
 * - **sessionStorage hint + CustomEvent** — Settings and Global panes,
 *   because a cold mount has nobody listening yet.
 * - **Pending React state** — projects and sessions, because the
 *   destination is an entity that has to be resolved before the section
 *   can show it.
 *
 * Each is reasonable alone. Together they mean "where do I send the
 * user" has four answers with four different capabilities, and a
 * destination degrades silently when it crosses from a richer transport
 * to a poorer one.
 *
 * # What this is NOT
 *
 * It is not a restore policy. Sections legitimately differ about what
 * to remember: Projects deliberately resets its tab and session
 * selection when the project changes, while Activities deliberately
 * remembers Stream/Usage/Cost. Forcing one rule on both would break one
 * of them, so `Destination` describes *where*, never *what to restore*.
 */
export type Destination = {
  /** Registry section id — the first level of navigation. */
  section: string;
  /**
   * The entity this destination is *about*, when it is about one.
   *
   * Kept separate from the path because it has a different lifetime: a
   * project or session id can stop existing between the moment a
   * notification is written and the moment it is clicked, and the
   * consumer must be able to fall back to the section rather than
   * treating the whole address as invalid.
   */
  target?: DestinationTarget;
} & (
  | { tab?: undefined; sub?: undefined }
  /**
   * `sub` requires `tab`. Without this, `{ section, sub }` encodes to
   * `section/sub` and decodes back as a TAB — the address silently
   * moves up a level and lands somewhere real, which is the worst
   * failure shape for a deep link. The illegal state is a type error
   * rather than a runtime check because every producer is ours.
   */
  | {
      /** Second level: a tab within the section, or a Settings pane. */
      tab: string;
      /** Third level: a sub-route within the tab (`repair`, `cleanup`). */
      sub?: string;
    }
);

export interface DestinationTarget {
  kind: "project" | "session" | "account" | "board";
  id: string;
}

/**
 * Section ids that shipped under an older name.
 *
 * These are **compatibility contracts**, not aliases we may retire: they
 * are the values already written into a user's `localStorage`, so a
 * decode that rejects them silently drops that user back to Accounts.
 * The registry still uses the legacy id as its canonical id in each
 * case, so this map is currently identity — it exists so the A3/A1
 * renames have somewhere to land without a second lookup appearing
 * elsewhere.
 */
export const LEGACY_SECTION_IDS: Readonly<Record<string, string>> = {
  events: "events",
  "third-party": "third-party",
  automations: "automations",
  "shared-memory": "shared-memory",
};

/** `section/tab/sub` — the canonical wire form. */
export function encodeDestination(d: Destination): string {
  return [d.section, d.tab, d.sub].filter(Boolean).join("/");
}

/**
 * Parse a destination from either wire form.
 *
 * Accepts `section/tab/sub` and the colon form the app menu and tray
 * emit (`section:tab:sub`), because those two serialise through an OS
 * API that only carries a string. Returns `null` for an unknown
 * section rather than guessing — a wrong section is worse than none,
 * and every caller already has a sensible fallback.
 */
export function decodeDestination(raw: string): Destination | null {
  const parts = raw
    .trim()
    .split(/[/:]/)
    .map((p) => p.trim())
    .filter(Boolean);
  if (parts.length === 0) return null;

  const section = LEGACY_SECTION_IDS[parts[0]] ?? parts[0];
  if (!sectionIds.includes(section)) return null;

  // Built in one expression: the type makes `sub` require `tab`, so
  // assigning them one at a time has no legal intermediate state.
  if (!parts[1]) return { section };
  return parts[2]
    ? { section, tab: parts[1], sub: parts[2] }
    : { section, tab: parts[1] };
}

/** Do two destinations name the same place? Ignores `target`. */
export function sameDestination(a: Destination, b: Destination): boolean {
  return a.section === b.section && a.tab === b.tab && a.sub === b.sub;
}

/**
 * The id a stored section value means today.
 *
 * Restore paths read a raw string out of `localStorage` and check it
 * against the current registry, silently falling back to Accounts when
 * it does not match. That is correct for a stale id from an older
 * build and *wrong* for a renamed one: the user is dropped somewhere
 * else and their saved position is gone with no way to tell which
 * happened.
 *
 * Every restore path goes through here so a rename has exactly one
 * place to land. The map is identity today — the point is that it
 * exists before the renames, not after.
 */
export function canonicalSectionId(stored: string | null | undefined): string | null {
  if (!stored) return null;
  return LEGACY_SECTION_IDS[stored] ?? stored;
}

