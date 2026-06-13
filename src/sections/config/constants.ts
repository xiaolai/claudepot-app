/**
 * Cross-section constants for the Config page.
 *
 * Lives outside `ConfigSection.tsx` so modules that need to *talk to*
 * Config (App.tsx's cross-section hop, ProjectDetail's "Open in
 * Config" affordance) don't have to eagerly import the ~50 KB
 * ConfigSection bundle just to know the anchor storage key and the
 * virtual sub-route for Effective Settings.
 */

import { CONFIG_ANCHOR_KEY } from "../../lib/storageKeys";

export const CONFIG_ANCHOR_STORAGE_KEY = CONFIG_ANCHOR_KEY;

export const EFFECTIVE_SETTINGS_ROUTE = "virtual:effective-settings";
export const EFFECTIVE_MCP_ROUTE = "virtual:effective-mcp";
