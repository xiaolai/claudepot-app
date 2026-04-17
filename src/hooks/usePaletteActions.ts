import { useMemo } from "react";
import type { AccountSummary, AppStatus } from "../types";

export interface PaletteAction {
  id: string;
  label: string;
  detail?: string;
  iconName: "terminal" | "monitor" | "user-plus" | "refresh-cw" | "trash";
  category: "switch" | "action";
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
}) {
  const { accounts, status, onSwitchCli, onSwitchDesktop, onAdd, onRefresh, onRemove } = opts;

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
      if (!a.is_desktop_active && a.has_desktop_profile && status.desktop_installed) {
        items.push({
          id: `desk-${a.uuid}`,
          label: `Switch Desktop to ${a.email}`,
          detail: a.org_name ?? "personal",
          iconName: "monitor",
          category: "switch",
          onSelect: () => onSwitchDesktop(a),
        });
      }
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
  }, [accounts, status, onSwitchCli, onSwitchDesktop, onAdd, onRefresh, onRemove]);

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
