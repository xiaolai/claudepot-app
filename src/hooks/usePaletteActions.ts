import { useMemo } from "react";
import type { AccountSummary, AppStatus } from "../types";

export interface PaletteAction {
  id: string;
  label: string;
  detail?: string;
  iconName:
    | "terminal"
    | "monitor"
    | "user-plus"
    | "refresh-cw"
    | "trash"
    | "folder"
    | "wrench"
    | "settings";
  category: "switch" | "action" | "navigate";
  disabled?: boolean;
  onSelect: () => void;
}

function fuzzyMatch(query: string, text: string): boolean {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  if (t.includes(q)) return true;
  let qi = 0;
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] === q[qi]) qi++;
  }
  return qi === q.length;
}

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
  /** Jump to a top-level section — "accounts", "projects", "settings". */
  onNavigate?: (section: string, subRoute?: string | null) => void;
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
  } = opts;

  const actions = useMemo(() => {
    const items: PaletteAction[] = [];
    for (const a of accounts) {
      if (!a.is_cli_active && a.credentials_healthy) {
        items.push({
          id: `cli-${a.uuid}`,
          label: `Switch CLI to ${a.email}`,
          detail: a.org_name ?? "personal",
          iconName: "terminal",
          category: "switch",
          onSelect: () => onSwitchCli(a),
        });
      }
    }
    for (const a of accounts) {
      // desktop_profile_on_disk is the disk-truth field; gates on
      // the actual snapshot existing so the switch won't error at
      // restore. See plan v2 D18.
      if (!a.is_desktop_active && a.desktop_profile_on_disk && status.desktop_installed) {
        items.push({
          id: `desk-${a.uuid}`,
          label: `Switch Desktop to ${a.email}`,
          detail: a.org_name ?? "personal",
          iconName: "monitor",
          category: "switch",
          onSelect: () => onSwitchDesktop(a),
        });
      }
      // Bind live Desktop into this account — the "no profile yet"
      // remedy. Shows whenever Desktop is installed and this account
      // doesn't already have a snapshot; the backend verifies the
      // live /profile email matches and errors cleanly otherwise.
      if (onAdoptDesktop && !a.desktop_profile_on_disk && status.desktop_installed) {
        items.push({
          id: `adopt-${a.uuid}`,
          label: `Bind current Desktop session to ${a.email}`,
          detail: a.org_name ?? "personal",
          iconName: "monitor",
          category: "action",
          onSelect: () => onAdoptDesktop(a),
        });
      }
    }
    if (status.desktop_installed && onClearDesktop) {
      items.push({
        id: "desktop-clear",
        label: "Sign Desktop out",
        iconName: "trash",
        category: "action",
        onSelect: onClearDesktop,
      });
    }
    if (status.desktop_installed && onLaunchDesktop) {
      items.push({
        id: "desktop-launch",
        label: "Launch Claude Desktop",
        iconName: "monitor",
        category: "action",
        onSelect: onLaunchDesktop,
      });
    }
    if (onNavigate) {
      items.push({
        id: "nav-projects",
        label: "Open Projects",
        iconName: "folder",
        category: "navigate",
        onSelect: () => onNavigate("projects"),
      });
      items.push({
        id: "nav-maintenance",
        label: "Open Maintenance",
        detail: "Clean + Repair",
        iconName: "wrench",
        category: "navigate",
        onSelect: () => onNavigate("projects", "maintenance"),
      });
      items.push({
        id: "nav-settings",
        label: "Open Settings",
        iconName: "settings",
        category: "navigate",
        onSelect: () => onNavigate("settings"),
      });
    }
    items.push({ id: "add", label: "Add account", iconName: "user-plus", category: "action", onSelect: onAdd });
    items.push({ id: "refresh", label: "Refresh all", iconName: "refresh-cw", category: "action", onSelect: onRefresh });
    for (const a of accounts) {
      items.push({
        id: `rm-${a.uuid}`,
        label: `Remove ${a.email}`,
        detail: a.org_name ?? "personal",
        iconName: "trash",
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
  ]);

  return {
    actions,
    filter: (query: string) => {
      if (!query.trim()) return actions;
      return actions.filter(
        (a) => fuzzyMatch(query, a.label) || (a.detail && fuzzyMatch(query, a.detail)),
      );
    },
  };
}
