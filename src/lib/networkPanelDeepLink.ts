/**
 * Cross-section deep-link contract. Named for its first caller — the
 * NetworkUnreachablePanel — but now also carries the ⌘K palette's
 * links into a Settings pane or a Global tab. Callers live in the App
 * shell and in `usePaletteActions`; the targets they navigate to
 * (Third-parties, Settings, Global) are lazy-mounted sections. We need
 * both:
 *
 * 1. A cold-mount path — section is not mounted yet; reads
 *    `sessionStorage` on mount.
 * 2. A hot-mount path — section is already the active section; a
 *    mounted listener picks up a `window` CustomEvent.
 *
 * Both paths have to work because `setSection` is a no-op when the
 * caller is already on the target section. Without the event path,
 * clicking "Use a provider" while already on Providers does
 * nothing (a real bug found in audit).
 *
 * Keys + event names live here (not at the call sites) so a typo or
 * rename fails loudly with a TypeScript error, not silently.
 */

// sessionStorage keys — cold-mount path. Declared centrally in
// lib/storageKeys.ts; re-exported here so call sites keep their
// established import path.
import {
  DEEPLINK_OPEN_ADD_ROUTE_KEY,
  DEEPLINK_FROM_NETWORK_PANEL_KEY,
  DEEPLINK_SETTINGS_TAB_KEY,
  DEEPLINK_GLOBAL_TAB_KEY,
} from "./storageKeys";

export const STORAGE_KEY_OPEN_ADD_ROUTE = DEEPLINK_OPEN_ADD_ROUTE_KEY;
export const STORAGE_KEY_FROM_NETWORK_PANEL =
  DEEPLINK_FROM_NETWORK_PANEL_KEY;
export const STORAGE_KEY_SETTINGS_TAB = DEEPLINK_SETTINGS_TAB_KEY;
export const STORAGE_KEY_GLOBAL_TAB = DEEPLINK_GLOBAL_TAB_KEY;

// Window CustomEvent names — hot-mount path.
export const EVENT_OPEN_ADD_ROUTE = "claudepot:openAddRoute";
export const EVENT_SETTINGS_TAB = "claudepot:settingsTab";
export const EVENT_GLOBAL_TAB = "claudepot:globalTab";

/** Detail payload for the settings-tab event. */
export interface SettingsTabEventDetail {
  tab: string;
}

/** Detail payload for the global-tab event. */
export interface GlobalTabEventDetail {
  tab: string;
}

/**
 * Trigger the "open Add Route" deep-link. Called from the panel's
 * onUseProvider handler. Sets sessionStorage AND dispatches the
 * CustomEvent so both cold-mount and hot-mount consumers fire.
 */
export function triggerOpenAddRoute(): void {
  try {
    sessionStorage.setItem(STORAGE_KEY_OPEN_ADD_ROUTE, "1");
    sessionStorage.setItem(STORAGE_KEY_FROM_NETWORK_PANEL, "1");
  } catch {
    // sessionStorage unavailable (private browsing, quota, disabled).
    // The event path still fires; cold-mount degrades gracefully.
  }
  window.dispatchEvent(new CustomEvent(EVENT_OPEN_ADD_ROUTE));
}

/**
 * Trigger the "switch Settings to network tab" deep-link. Called
 * from the panel's onConfigureProxy handler.
 */
export function triggerSettingsTab(tab: string): void {
  try {
    sessionStorage.setItem(STORAGE_KEY_SETTINGS_TAB, tab);
  } catch {
    // Same swallow as above.
  }
  window.dispatchEvent(
    new CustomEvent<SettingsTabEventDetail>(EVENT_SETTINGS_TAB, {
      detail: { tab },
    }),
  );
}

/**
 * Cold-mount consumer for the "open Add Route" deep-link. Returns
 * true if the hint was set (and clears it). Call from a `useState`
 * initializer in ThirdPartySection.
 */
export function consumeOpenAddRouteHint(): boolean {
  try {
    if (sessionStorage.getItem(STORAGE_KEY_OPEN_ADD_ROUTE) === "1") {
      sessionStorage.removeItem(STORAGE_KEY_OPEN_ADD_ROUTE);
      return true;
    }
  } catch {
    // Same swallow.
  }
  return false;
}

/**
 * Cold-mount consumer for "settings tab" hint. Returns the tab
 * string and clears the hint, or null if no hint was set.
 */
export function consumeSettingsTabHint(): string | null {
  return consumeHint(STORAGE_KEY_SETTINGS_TAB);
}

/**
 * Trigger the "switch Global to this tab" deep-link, used by the ⌘K
 * palette's "Open Global → Updates" style entries.
 *
 * This deliberately does NOT ride the section `subRoute`. That channel
 * is persisted per section, so writing a tab id into it both clobbers
 * whatever `node:<id>` Config route was stored there and outlives the
 * one navigation it was meant to describe. A transient hint expresses
 * "select this tab, once" without owning the section's saved state.
 */
export function triggerGlobalTab(tab: string): void {
  try {
    sessionStorage.setItem(STORAGE_KEY_GLOBAL_TAB, tab);
  } catch {
    // Same swallow as above.
  }
  window.dispatchEvent(
    new CustomEvent<GlobalTabEventDetail>(EVENT_GLOBAL_TAB, {
      detail: { tab },
    }),
  );
}

/** Cold-mount consumer for the "global tab" hint. */
export function consumeGlobalTabHint(): string | null {
  return consumeHint(STORAGE_KEY_GLOBAL_TAB);
}

/** Read-and-clear a one-shot sessionStorage hint. */
function consumeHint(key: string): string | null {
  try {
    const v = sessionStorage.getItem(key);
    if (v) {
      sessionStorage.removeItem(key);
      return v;
    }
  } catch {
    // Same swallow.
  }
  return null;
}

/**
 * Read-without-clear for the "from network panel" breadcrumb. The
 * RouteForm reads this to decide whether to highlight China-reachable
 * presets. The breadcrumb is cleared when the modal closes (see
 * `clearFromNetworkPanelBreadcrumb`), not when read — a remount
 * during the same modal session should keep the highlight.
 */
export function readFromNetworkPanelBreadcrumb(): boolean {
  try {
    return sessionStorage.getItem(STORAGE_KEY_FROM_NETWORK_PANEL) === "1";
  } catch {
    return false;
  }
}

export function clearFromNetworkPanelBreadcrumb(): void {
  try {
    sessionStorage.removeItem(STORAGE_KEY_FROM_NETWORK_PANEL);
  } catch {
    // Same swallow.
  }
}
