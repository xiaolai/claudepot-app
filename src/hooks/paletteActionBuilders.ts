import { NF } from "../icons";
import { sections } from "../sections/registry";
import { SETTINGS_PANES } from "../sections/settings/panes";
import { GLOBAL_TABS } from "../sections/global/tabs";
import {
  triggerGlobalTab,
  triggerSettingsTab,
} from "../lib/networkPanelDeepLink";
import type { PaletteAction } from "./usePaletteActions";
import type { AccountSummary, AppStatus } from "../types";

/* ── Action builders ────────────────────────────────────────────────
 *
 * One builder per group, each a pure function of the state it needs.
 * These were a single 150-line `useMemo` body, which made the
 * visibility gate for any one row hard to read against the others.
 * Composition order is irrelevant to the rendered order — `filter`
 * sorts by category — so a builder can be read in isolation.
 */

export function buildSwitchActions(o: {
  accounts: AccountSummary[];
  status: AppStatus;
  onSwitchCli: (a: AccountSummary) => void;
  onSwitchDesktop: (a: AccountSummary) => void;
}): PaletteAction[] {
  const items: PaletteAction[] = [];
  for (const a of o.accounts) {
    if (!a.is_cli_active && a.credentials_healthy) {
      items.push({
        id: `cli-${a.uuid}`,
        label: `Switch CLI to ${a.email}`,
        detail: a.org_name ?? "personal",
        glyph: NF.terminal,
        category: "switch",
        onSelect: () => o.onSwitchCli(a),
      });
    }
  }
  for (const a of o.accounts) {
    // desktop_profile_on_disk is the disk-truth field; gates on the
    // actual snapshot existing so the switch won't error at restore.
    // See plan v2 D18.
    if (
      !a.is_desktop_active &&
      a.desktop_profile_on_disk &&
      o.status.desktop_installed
    ) {
      items.push({
        id: `desk-${a.uuid}`,
        label: `Switch Desktop to ${a.email}`,
        detail: a.org_name ?? "personal",
        glyph: NF.desktop,
        category: "switch",
        onSelect: () => o.onSwitchDesktop(a),
      });
    }
  }
  return items;
}

export function buildNavigateActions(
  onNavigate?: (section: string, subRoute?: string | null) => void,
): PaletteAction[] {
  if (!onNavigate) return [];
  const items: PaletteAction[] = [];

  // Every top-level section, straight off the registry that the
  // sidebar and ⌘1..⌘9 already read. Hardcoding a subset here is what
  // left six of the nine sections unreachable from ⌘K.
  for (const s of sections) {
    items.push({
      id: `nav-${s.id}`,
      label: `Open ${s.label}`,
      glyph: s.glyph,
      category: "navigate",
      onSelect: () => onNavigate(s.id),
    });
  }

  // Projects → Maintenance keeps its own entry: it's a sub-view with
  // no section of its own, and "clean"/"repair" are things users
  // search for by name.
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
        // Set the hint before navigating: SettingsSection reads it in
        // a useState initializer on cold mount, and listens for the
        // paired event when it's already mounted.
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
      onSelect: () => {
        // Transient hint, not a sub-route: `subRoute` is persisted per
        // section and doubles as the Config tree's saved selection, so
        // a tab id written there would both clobber that selection and
        // outlive this one navigation.
        triggerGlobalTab(tab.id);
        onNavigate("global");
      },
    });
  }
  return items;
}

export function buildGeneralActions(o: {
  onAdd: () => void;
  onRefresh: () => void;
  onToggleTheme?: () => void;
  onShowShortcuts?: () => void;
}): PaletteAction[] {
  const items: PaletteAction[] = [
    {
      id: "add",
      label: "Add account",
      glyph: NF.userPlus,
      category: "action",
      onSelect: o.onAdd,
    },
    {
      id: "refresh",
      label: "Refresh all",
      keywords: ["reload", "sync"],
      glyph: NF.refresh,
      category: "action",
      onSelect: o.onRefresh,
    },
  ];
  if (o.onToggleTheme) {
    items.push({
      id: "toggle-theme",
      label: "Toggle theme",
      detail: "Light / dark",
      keywords: ["dark mode", "light mode", "appearance"],
      glyph: NF.moon,
      category: "action",
      onSelect: o.onToggleTheme,
    });
  }
  if (o.onShowShortcuts) {
    items.push({
      id: "shortcuts",
      label: "Show keyboard shortcuts",
      detail: "⌘ /",
      glyph: NF.help,
      category: "action",
      onSelect: o.onShowShortcuts,
    });
  }
  return items;
}

export function buildDesktopActions(o: {
  accounts: AccountSummary[];
  status: AppStatus;
  onAdoptDesktop?: (a: AccountSummary) => void;
  onClearDesktop?: () => void;
  onLaunchDesktop?: () => void;
}): PaletteAction[] {
  if (!o.status.desktop_installed) return [];
  const items: PaletteAction[] = [];
  const { onAdoptDesktop, onClearDesktop, onLaunchDesktop } = o;

  if (onLaunchDesktop) {
    items.push({
      id: "desktop-launch",
      label: "Launch Claude Desktop",
      glyph: NF.desktop,
      category: "action",
      onSelect: onLaunchDesktop,
    });
  }
  // Bind live Desktop into an account — the "no profile yet" remedy.
  // Shows for any account without a snapshot; the backend verifies the
  // live /profile email matches and errors cleanly otherwise.
  if (onAdoptDesktop) {
    for (const a of o.accounts) {
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
  if (onClearDesktop) {
    items.push({
      id: "desktop-clear",
      label: "Sign Desktop out",
      keywords: ["log out", "disconnect"],
      glyph: NF.trash,
      category: "action",
      onSelect: onClearDesktop,
    });
  }
  return items;
}

export function buildAccountRemovalActions(
  accounts: AccountSummary[],
  onRemove: (a: AccountSummary) => void,
): PaletteAction[] {
  return accounts.map((a) => ({
    id: `rm-${a.uuid}`,
    label: `Remove ${a.email}`,
    detail: a.org_name ?? "personal",
    glyph: NF.trash,
    category: "action" as const,
    onSelect: () => onRemove(a),
  }));
}
