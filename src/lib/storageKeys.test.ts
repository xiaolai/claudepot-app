import { describe, expect, it } from "vitest";
import * as keys from "./storageKeys";

// Golden values. These strings are what's already persisted on
// users' disks — a failing assertion here means a compat break, not
// a test to update. See the module header in storageKeys.ts.
const GOLDEN: Record<string, string> = {
  SECTION_ACTIVE_KEY: "claudepot.activeSection",
  SECTION_START_KEY: "claudepot.startSection",
  SECTION_SUBROUTE_KEY_PREFIX: "claudepot.subRoute.",
  THEME_KEY: "cp-theme",
  LOCALE_KEY: "claudepot.locale",
  DEV_MODE_KEY: "cp-dev-mode",
  SIDEBAR_COLLAPSED_KEY: "cp-sidebar-collapsed",
  DISMISSED_ISSUES_KEY: "claudepot.dismissedIssues",
  NETWORK_GATE_DISMISSED_KEY: "claudepot.networkGate.dismissed",
  EVENTS_TAB_KEY: "claudepot.events.tab",
  GLOBAL_TAB_KEY: "claudepot.global.tab",
  CONFIG_ANCHOR_KEY: "claudepot.config.anchor",
  UPDATE_AUTO_CHECK_KEY: "claudepot.update.autoCheckEnabled",
  UPDATE_CHECK_FREQ_KEY: "claudepot.update.checkFrequency",
  UPDATE_LAST_CHECKED_KEY: "claudepot.update.lastCheckedAt",
  UPDATE_SKIP_VERSION_KEY: "claudepot.update.skipVersion",
  DEEPLINK_OPEN_ADD_ROUTE_KEY: "claudepot.deepLink.openAddRoute",
  DEEPLINK_FROM_NETWORK_PANEL_KEY: "claudepot.deepLink.fromNetworkPanel",
  DEEPLINK_SETTINGS_TAB_KEY: "claudepot.deepLink.settingsTab",
  DEEPLINK_GLOBAL_TAB_KEY: "claudepot.deepLink.globalTab",
};

describe("storageKeys — byte-for-byte compat contract", () => {
  it("every exported key matches its golden value", () => {
    // Only the string constants are the on-disk contract. Key
    // *builders* are checked separately below — comparing the whole
    // module would make adding one look like a compat break.
    const constants = Object.fromEntries(
      Object.entries(keys).filter(([, v]) => typeof v === "string"),
    );
    expect(constants).toEqual(GOLDEN);
  });

  it("optionalSectionKey builds the persisted per-section key", () => {
    // Same contract as the constants: this string is already on disk,
    // and `sections/registry.tsx` reads it through this builder rather
    // than a hand-copied literal, so a change here silently breaks the
    // preload guards as well as the toggle.
    expect(keys.optionalSectionKey("boards")).toBe("claudepot.optional.boards");
  });
});
