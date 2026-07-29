import { NF, type NfIcon } from "../../icons";

/**
 * Settings pane metadata — the single source of truth for the
 * Settings sub-nav AND for the ⌘K palette's deep links into it.
 *
 * This lives in its own module (no components, no JSX) on purpose:
 * `usePaletteActions` needs the pane list to build "Settings →
 * Retention" style targets, and importing `SettingsSection` for it
 * would drag the whole lazy Settings chunk back into the main
 * bundle, undoing the code-split in `sections/registry.tsx`.
 *
 * `SettingsPaneId` is derived from the table rather than hand-written
 * beside it — the two used to be separate declarations that had to be
 * edited in lock-step.
 */
export interface SettingsPaneDef {
  id: string;
  label: string;
  glyph: NfIcon;
  group: "core" | "advanced";
  /**
   * Extra ⌘K search terms for panes whose label isn't what a user
   * would type. "Retention" is the thing you look for when you search
   * "delete transcripts"; the label alone would never match.
   */
  keywords?: readonly string[];
}

export const SETTINGS_PANES = [
  { id: "general", label: "General", glyph: NF.sliders, group: "core",
    keywords: ["preferences", "launch", "startup"] },
  { id: "appearance", label: "Appearance", glyph: NF.sun, group: "core",
    keywords: ["theme", "dark", "light", "font"] },
  { id: "notifications", label: "Notifications", glyph: NF.bell, group: "core",
    keywords: ["alerts", "banners", "toasts"] },
  { id: "network", label: "Network", glyph: NF.globe, group: "core",
    keywords: ["proxy", "offline", "connection"] },
  { id: "rotation", label: "Rotation", glyph: NF.refresh, group: "core",
    keywords: ["auto-rotate", "swap", "threshold", "quota"] },
  // Retention = CC's `cleanupPeriodDays`, the only CC setting that
  // destroys user data. "core", not "advanced": a control that exists
  // to prevent silent data loss is worthless if the user has to go
  // looking for it, and CC's own UI never mentions the setting at all.
  { id: "retention", label: "Retention", glyph: NF.archive, group: "core",
    keywords: ["transcripts", "delete", "expiry", "cleanupPeriodDays"] },
  // Health = CC self-diagnostic (scrapes `claude doctor`). Sits in
  // "core" because the pill in WindowChrome points here directly;
  // hiding it under Advanced would make the deep-link surface
  // inconsistent. Distinct from "Diagnostics" below, which is
  // Claudepot's own self-check (platform, accounts, data dir).
  { id: "health", label: "Health", glyph: NF.shield, group: "core",
    keywords: ["doctor", "claude doctor"] },
  { id: "mcp", label: "MCP", glyph: NF.server, group: "core",
    keywords: ["model context protocol", "servers", "install"] },
  { id: "cleanup", label: "Cleanup", glyph: NF.trash, group: "advanced",
    keywords: ["prune", "trash", "rebuild index", "sessions"] },
  { id: "protected", label: "Protected paths", glyph: NF.shield,
    group: "advanced", keywords: ["exclude", "safelist"] },
  { id: "github", label: "GitHub", glyph: NF.key, group: "advanced",
    keywords: ["token", "pat", "export"] },
  { id: "locks", label: "Locks", glyph: NF.lock, group: "advanced",
    keywords: ["stale lock", "break lock"] },
  { id: "diagnostics", label: "Diagnostics", glyph: NF.wrench,
    group: "advanced", keywords: ["self-check", "data dir", "platform"] },
  { id: "about", label: "About", glyph: NF.info, group: "advanced",
    keywords: ["version", "license", "credits"] },
] as const satisfies readonly SettingsPaneDef[];

/** One row of the table, with its literal `id` preserved. */
export type SettingsPane = (typeof SETTINGS_PANES)[number];

export type SettingsPaneId = SettingsPane["id"];

export function isSettingsPaneId(v: string): v is SettingsPaneId {
  return SETTINGS_PANES.some((p) => p.id === v);
}
