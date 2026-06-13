/**
 * Every localStorage / sessionStorage key Claudepot writes, in one
 * place. The VALUES are a compatibility contract — they are what's
 * already on users' disks — so they must never change, even where
 * the spelling is awkward:
 *
 *   - Two naming schemes coexist: the older `cp-*` family (theme,
 *     dev mode, sidebar) and the `claudepot.*` family (everything
 *     else). New keys adopt the `claudepot.` prefix by convention;
 *     the `cp-*` trio stays as-is for compat.
 *   - Section ids stored under SECTION_ACTIVE / SECTION_START are
 *     also legacy-locked: "events" (label "Activities"),
 *     "automations" (label "Agents"), "third-party" (label
 *     "Providers"). See `src/sections/registry.tsx`.
 *
 * Known duplicate not yet centralized: SettingsSection.tsx carries
 * the `claudepot.startSection` literal inline (lines ~286/329); it
 * is scheduled for its own decomposition pass — point it here when
 * that lands.
 */

// --- Section navigation (owner: src/hooks/useSection.ts) -----------
/** Last section the user navigated to. */
export const SECTION_ACTIVE_KEY = "claudepot.activeSection";
/** Explicit "Open on launch" preference; wins over SECTION_ACTIVE_KEY. */
export const SECTION_START_KEY = "claudepot.startSection";
/** Per-section sub-route, stored as `claudepot.subRoute.<sectionId>`. */
export const SECTION_SUBROUTE_KEY_PREFIX = "claudepot.subRoute.";

// --- Shell chrome ---------------------------------------------------
/** Explicit theme override (owner: src/hooks/useTheme.ts). */
export const THEME_KEY = "cp-theme";
/** Developer-mode toggle (owner: src/hooks/useDevMode.ts). */
export const DEV_MODE_KEY = "cp-dev-mode";
/** Sidebar collapse state (owner: src/hooks/useSidebarCollapsed.ts). */
export const SIDEBAR_COLLAPSED_KEY = "cp-sidebar-collapsed";

// --- Banners / issue snoozes ----------------------------------------
/** Snoozed status-issue ids (owner: src/hooks/useDismissedIssues.ts). */
export const DISMISSED_ISSUES_KEY = "claudepot.dismissedIssues";
/** sessionStorage: network panel dismissed for this session
 *  (owner: src/hooks/useNetworkGate.ts). */
export const NETWORK_GATE_DISMISSED_KEY = "claudepot.networkGate.dismissed";

// --- Per-section UI state -------------------------------------------
/** Activities tab (owner: src/sections/EventsSection.tsx). */
export const EVENTS_TAB_KEY = "claudepot.events.tab";
/** Global tab (owner: src/sections/GlobalSection.tsx). */
export const GLOBAL_TAB_KEY = "claudepot.global.tab";
/** Config anchor (owner: src/sections/config/constants.ts). */
export const CONFIG_ANCHOR_KEY = "claudepot.config.anchor";

// --- Updates (owner: src/providers/UpdateProvider.tsx) ---------------
export const UPDATE_AUTO_CHECK_KEY = "claudepot.update.autoCheckEnabled";
export const UPDATE_CHECK_FREQ_KEY = "claudepot.update.checkFrequency";
export const UPDATE_LAST_CHECKED_KEY = "claudepot.update.lastCheckedAt";
export const UPDATE_SKIP_VERSION_KEY = "claudepot.update.skipVersion";

// --- Deep links (sessionStorage; owner: src/lib/networkPanelDeepLink.ts)
export const DEEPLINK_OPEN_ADD_ROUTE_KEY = "claudepot.deepLink.openAddRoute";
export const DEEPLINK_FROM_NETWORK_PANEL_KEY =
  "claudepot.deepLink.fromNetworkPanel";
export const DEEPLINK_SETTINGS_TAB_KEY = "claudepot.deepLink.settingsTab";
