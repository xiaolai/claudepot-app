// Localized display names for notification categories and their
// groups. Core's `display_meta()` ships English labels over IPC; the
// UI keys off the stable category *id* (and group string) into the
// shell catalog instead, falling back to the IPC-shipped English when
// a catalog entry is missing — so a new core category degrades to
// English rather than rendering a raw key (i18n plan §2.4). The
// fixture test in `types.test.ts` makes that fallback unreachable in
// practice by asserting catalog completeness against
// `__fixtures__/categories.fixture.json`.

import { i18n } from "../i18n";

// Typed t() rejects computed keys by design; this module is the one
// sanctioned dynamic lookup for category ids.
const tShell = (key: string): string =>
  (i18n.t as unknown as (k: string, o?: object) => string)(key, {
    ns: "shell",
  });

const existsShell = (key: string): boolean =>
  i18n.exists(key, { ns: "shell" });

/** Localized label for a category row; falls back to the IPC label. */
export function categoryLabel(meta: { id: string; label: string }): string {
  const key = `notifCategories.${meta.id}`;
  return existsShell(key) ? tShell(key) : meta.label;
}

const GROUP_SLUGS: Record<string, string> = {
  Setup: "setup",
  "Live work": "liveWork",
  Actions: "actions",
  Background: "background",
};

/** Localized label for a category group header; falls back to the
 *  wire string for a group core adds later. */
export function categoryGroupLabel(group: string): string {
  const slug = GROUP_SLUGS[group];
  if (!slug) return group;
  const key = `notifGroups.${slug}`;
  return existsShell(key) ? tShell(key) : group;
}
