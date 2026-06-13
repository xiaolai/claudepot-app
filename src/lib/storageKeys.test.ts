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
};

describe("storageKeys — byte-for-byte compat contract", () => {
  it("every exported key matches its golden value", () => {
    expect({ ...keys }).toEqual(GOLDEN);
  });
});
