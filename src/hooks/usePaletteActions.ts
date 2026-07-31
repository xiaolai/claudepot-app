import { useMemo } from "react";
import { useEnabledSections } from "./useEnabledSections";
import { type NfIcon } from "../icons";
import { scoreFields } from "../lib/paletteScore";
import {
  buildAccountRemovalActions,
  buildDesktopActions,
  buildGeneralActions,
  buildNavigateActions,
  buildSwitchActions,
} from "./paletteActionBuilders";
import type { AccountSummary, AppStatus } from "../types";

/** Declared below by `CATEGORY_ORDER`, which is also the render order. */
export type PaletteCategory = (typeof CATEGORY_ORDER)[number];

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
const CATEGORY_ORDER = ["switch", "navigate", "action"] as const;

/**
 * Derived, not declared. A hand-written rank map beside the order list
 * is two encodings of one fact: let them disagree and `filter()`
 * sorts into an order the renderer doesn't group by, which is exactly
 * the two-orderings bug this module was restructured to prevent.
 */
const CATEGORY_RANK = Object.fromEntries(
  CATEGORY_ORDER.map((c, i) => [c, i]),
) as Record<PaletteCategory, number>;

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

  // Re-derived when optional sections change: an open palette
  // memoized before a toggle kept offering a section that had just
  // been hidden.
  const enabled = useEnabledSections();

  const actions = useMemo(
    () => [
      ...buildSwitchActions({ accounts, status, onSwitchCli, onSwitchDesktop }),
      ...buildNavigateActions(onNavigate),
      ...buildGeneralActions({ onAdd, onRefresh, onToggleTheme, onShowShortcuts }),
      ...buildDesktopActions({
        accounts, status, onAdoptDesktop, onClearDesktop, onLaunchDesktop,
      }),
      ...buildAccountRemovalActions(accounts, onRemove),
    ],
    [
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
    // Optional-section state: an open palette memoized before a toggle
    // kept offering a section that had just been hidden.
    enabled,
  ],
  );

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
