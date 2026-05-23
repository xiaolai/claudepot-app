/**
 * Cross-section contract for the NetworkUnreachablePanel's deep-link
 * buttons. The panel lives in the App shell; the targets it
 * navigates to (Third-parties, Settings → Network) are lazy-mounted
 * sections. We need both:
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

// sessionStorage keys — cold-mount path.
export const STORAGE_KEY_OPEN_ADD_ROUTE =
  "claudepot.deepLink.openAddRoute";
export const STORAGE_KEY_FROM_NETWORK_PANEL =
  "claudepot.deepLink.fromNetworkPanel";
export const STORAGE_KEY_SETTINGS_TAB = "claudepot.deepLink.settingsTab";

// Window CustomEvent names — hot-mount path.
export const EVENT_OPEN_ADD_ROUTE = "claudepot:openAddRoute";
export const EVENT_SETTINGS_TAB = "claudepot:settingsTab";

/** Detail payload for the settings-tab event. */
export interface SettingsTabEventDetail {
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
  try {
    const v = sessionStorage.getItem(STORAGE_KEY_SETTINGS_TAB);
    if (v) {
      sessionStorage.removeItem(STORAGE_KEY_SETTINGS_TAB);
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
