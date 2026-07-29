import { useMemo } from "react";
import { NF, type NfIcon } from "../icons";
import { sections } from "../sections/registry";
import { SETTINGS_PANES } from "../sections/settings/panes";
import { GLOBAL_TABS } from "../sections/global/tabs";
import { triggerSettingsTab } from "../lib/networkPanelDeepLink";
import { scoreFields } from "../lib/paletteScore";
import type { AccountSummary, AppStatus } from "../types";

export type PaletteCategory = "switch" | "navigate" | "action";

export interface PaletteAction {
  id: string;
  label: string;
  detail?: string;
  /** Extra match terms that never render — synonyms, file names. */
  keywords?: readonly string[];
  glyph: NfIcon;
  category: PaletteCategory;
  /**
   * Deep targets (a Settings pane, a Global tab) are hidden until the
   * user types. Listing all 28 of them on an empty query buries the
   * nine top-level sections and every account action under a wall of
   * sub-navigation.
   */
  deep?: boolean;
  onSelect: () => void;
}

/**
 * Display order. The palette renders one group per category in this
 * order, so sorting the action list by it makes the produced order
 * and the rendered order the same list — which is the invariant that
 * keeps a cursor index addressing the row the user actually sees.
 */
const CATEGORY_ORDER: readonly PaletteCategory[] = [
  "switch",
  "navigate",
  "action",
];

const CATEGORY_RANK: Record<PaletteCategory, number> = {
  switch: 0,
  navigate: 1,
  action: 2,
};

export const PALETTE_CATEGORY_LABELS: Record<PaletteCategory, string> = {
  switch: "Quick Switch",
  navigate: "Navigate",
  action: "Actions",
};

export { CATEGORY_ORDER };

export function usePaletteActions(opts: {
  accounts: AccountSummary[];
  status: AppStatus;
  onSwitchCli: (a: AccountSummary) => void;
  onSwitchDesktop: (a: AccountSummary) => void;
  onAdd: () => void;
  onRefresh: () => void;
  onRemove: (a: AccountSummary) => void;
  /** Bind current Desktop session to this account. Phase 3+. */
  onAdoptDesktop?: (a: AccountSummary) => void;
  /** Sign Desktop out. */
  onClearDesktop?: () => void;
  /** Launch Claude Desktop. */
  onLaunchDesktop?: () => void;
  /** Jump to a top-level section, optionally with a sub-route. */
  onNavigate?: (section: string, subRoute?: string | null) => void;
  /** Open the global keyboard shortcuts reference modal. */
  onShowShortcuts?: () => void;
  /** Flip the light/dark theme. */
  onToggleTheme?: () => void;
}) {
  const {
    accounts,
    status,
    onSwitchCli,
    onSwitchDesktop,
    onAdd,
    onRefresh,
    onRemove,
    onAdoptDesktop,
    onClearDesktop,
    onLaunchDesktop,
    onNavigate,
    onShowShortcuts,
    onToggleTheme,
  } = opts;

  const actions = useMemo(() => {
    const items: PaletteAction[] = [];

    // ---------- Quick Switch ----------
    for (const a of accounts) {
      if (!a.is_cli_active && a.credentials_healthy) {
        items.push({
          id: `cli-${a.uuid}`,
          label: `Switch CLI to ${a.email}`,
          detail: a.org_name ?? "personal",
          glyph: NF.terminal,
          category: "switch",
          onSelect: () => onSwitchCli(a),
        });
      }
    }
    for (const a of accounts) {
      // desktop_profile_on_disk is the disk-truth field; gates on
      // the actual snapshot existing so the switch won't error at
      // restore. See plan v2 D18.
      if (
        !a.is_desktop_active &&
        a.desktop_profile_on_disk &&
        status.desktop_installed
      ) {
        items.push({
          id: `desk-${a.uuid}`,
          label: `Switch Desktop to ${a.email}`,
          detail: a.org_name ?? "personal",
          glyph: NF.desktop,
          category: "switch",
          onSelect: () => onSwitchDesktop(a),
        });
      }
    }

    // ---------- Navigate ----------
    if (onNavigate) {
      // Every top-level section, straight off the registry that the
      // sidebar and ⌘1..⌘9 already read. Hardcoding a subset here is
      // what left six of the nine sections unreachable from ⌘K.
      for (const s of sections) {
        items.push({
          id: `nav-${s.id}`,
          label: `Open ${s.label}`,
          glyph: s.glyph,
          category: "navigate",
          onSelect: () => onNavigate(s.id),
        });
      }

      // Projects → Maintenance keeps its own entry: it's a sub-view
      // with no section of its own, and "clean"/"repair" are things
      // users search for by name.
      items.push({
        id: "nav-maintenance",
        label: "Open Projects → Maintenance",
        detail: "Clean + Repair",
        keywords: ["clean", "repair", "gc", "orphans"],
        glyph: NF.tools,
        category: "navigate",
        deep: true,
        onSelect: () => onNavigate("projects", "maintenance"),
      });

      for (const pane of SETTINGS_PANES) {
        items.push({
          id: `nav-settings-${pane.id}`,
          label: `Open Settings → ${pane.label}`,
          keywords: pane.keywords,
          glyph: pane.glyph,
          category: "navigate",
          deep: true,
          onSelect: () => {
            // Set the hint before navigating: SettingsSection reads it
            // in a useState initializer on cold mount, and listens for
            // the paired event when it's already mounted.
            triggerSettingsTab(pane.id);
            onNavigate("settings");
          },
        });
      }

      for (const tab of GLOBAL_TABS) {
        items.push({
          id: `nav-global-${tab.id}`,
          label: `Open Global → ${tab.label}`,
          keywords: tab.keywords,
          glyph: tab.glyph,
          category: "navigate",
          deep: true,
          onSelect: () => onNavigate("global", `tab:${tab.id}`),
        });
      }
    }

    // ---------- Actions ----------
    items.push({
      id: "add",
      label: "Add account",
      glyph: NF.userPlus,
      category: "action",
      onSelect: onAdd,
    });
    items.push({
      id: "refresh",
      label: "Refresh all",
      keywords: ["reload", "sync"],
      glyph: NF.refresh,
      category: "action",
      onSelect: onRefresh,
    });
    if (onToggleTheme) {
      items.push({
        id: "toggle-theme",
        label: "Toggle theme",
        detail: "Light / dark",
        keywords: ["dark mode", "light mode", "appearance"],
        glyph: NF.moon,
        category: "action",
        onSelect: onToggleTheme,
      });
    }
    if (onShowShortcuts) {
      items.push({
        id: "shortcuts",
        label: "Show keyboard shortcuts",
        detail: "⌘ /",
        glyph: NF.help,
        category: "action",
        onSelect: onShowShortcuts,
      });
    }
    if (status.desktop_installed && onLaunchDesktop) {
      items.push({
        id: "desktop-launch",
        label: "Launch Claude Desktop",
        glyph: NF.desktop,
        category: "action",
        onSelect: onLaunchDesktop,
      });
    }
    // Bind live Desktop into an account — the "no profile yet"
    // remedy. Shows whenever Desktop is installed and this account
    // doesn't already have a snapshot; the backend verifies the live
    // /profile email matches and errors cleanly otherwise.
    if (onAdoptDesktop && status.desktop_installed) {
      for (const a of accounts) {
        if (a.desktop_profile_on_disk) continue;
        items.push({
          id: `adopt-${a.uuid}`,
          label: `Bind current Desktop session to ${a.email}`,
          detail: a.org_name ?? "personal",
          glyph: NF.desktop,
          category: "action",
          onSelect: () => onAdoptDesktop(a),
        });
      }
    }
    if (status.desktop_installed && onClearDesktop) {
      items.push({
        id: "desktop-clear",
        label: "Sign Desktop out",
        keywords: ["log out", "disconnect"],
        glyph: NF.trash,
        category: "action",
        onSelect: onClearDesktop,
      });
    }
    for (const a of accounts) {
      items.push({
        id: `rm-${a.uuid}`,
        label: `Remove ${a.email}`,
        detail: a.org_name ?? "personal",
        glyph: NF.trash,
        category: "action",
        onSelect: () => onRemove(a),
      });
    }
    return items;
  }, [
    accounts,
    status,
    onSwitchCli,
    onSwitchDesktop,
    onAdd,
    onRefresh,
    onRemove,
    onAdoptDesktop,
    onClearDesktop,
    onLaunchDesktop,
    onNavigate,
    onShowShortcuts,
    onToggleTheme,
  ]);

  return {
    /**
     * Actions matching `query`, in the exact order the palette renders
     * them: grouped by category, best match first within each group.
     * An empty query keeps production order and hides deep targets.
     */
    filter: (query: string): PaletteAction[] => {
      const q = query.trim();
      if (!q) {
        return [...actions]
          .filter((a) => !a.deep)
          .sort((x, y) => CATEGORY_RANK[x.category] - CATEGORY_RANK[y.category]);
      }
      const scored: { action: PaletteAction; score: number }[] = [];
      for (const action of actions) {
        const score = scoreFields(q, action.label, [
          action.detail,
          ...(action.keywords ?? []),
        ]);
        if (score !== null) scored.push({ action, score });
      }
      scored.sort((x, y) => {
        const byCategory =
          CATEGORY_RANK[x.action.category] - CATEGORY_RANK[y.action.category];
        if (byCategory !== 0) return byCategory;
        return y.score - x.score;
      });
      return scored.map((s) => s.action);
    },
  };
}
