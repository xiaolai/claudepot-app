import type { Destination } from "./destination";
import { triggerGlobalTab, triggerSettingsTab } from "./networkPanelDeepLink";

/**
 * Send the user to a `Destination`, whatever level it names.
 *
 * # Why a dispatcher and not four call sites
 *
 * Reaching a second-level destination means a *different* mechanism per
 * section: Settings panes go through `triggerSettingsTab`, the Config
 * section's tabs through `triggerGlobalTab`, and a section with neither
 * just needs `setSection`. Every caller that wanted to deep-link had to
 * know which — so `useNotificationClickRouter` knew about sections and
 * not panes and silently dropped the pane, and the app-menu router
 * hand-rolled its own colon parse.
 *
 * One dispatcher means a caller supplies an address and does not need to
 * know how that section is reached. Adding a section with tabs means
 * registering its trigger here, once.
 *
 * # Cold mount vs hot mount
 *
 * Both triggers write a `sessionStorage` hint *and* dispatch an event,
 * because the target section may not be mounted yet when the navigation
 * starts. The hint covers the cold path (the section reads it in a
 * `useState` initializer), the event covers the hot one. Dropping either
 * half breaks exactly one of those two cases, which is the kind of bug
 * that only shows up on a fresh launch.
 */
export type SetSection = (id: string, subRoute?: string | null) => void;

/**
 * Sections whose second level is reached by a tab hint rather than by
 * `setSection`'s sub-route.
 *
 * Keyed by section id. A section absent from here has no tab level, and
 * a `Destination` naming a tab for it is a programming error the
 * dispatcher ignores rather than guesses at.
 */
const TAB_TRIGGERS: Readonly<Record<string, (tab: string) => void>> = {
  settings: triggerSettingsTab,
  config: triggerGlobalTab,
};

export function navigateTo(setSection: SetSection, dest: Destination): void {
  // The sub-route rides along with the section change so both land in
  // one update — `setSection(id, sub)` exists precisely so a deep link
  // does not flash the section's previous sub-route first.
  setSection(dest.section, dest.sub ?? null);
  if (!dest.tab) return;
  const trigger = TAB_TRIGGERS[dest.section];
  if (trigger) trigger(dest.tab);
}

/** Section ids that can receive a tab. Exported for tests and guards. */
export function sectionsWithTabs(): string[] {
  return Object.keys(TAB_TRIGGERS).sort();
}
