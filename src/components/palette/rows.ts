import {
  CATEGORY_ORDER,
  PALETTE_CATEGORY_LABELS,
  type PaletteAction,
} from "../../hooks/usePaletteActions";
import { MIN_SEARCH_QUERY } from "../../lib/paletteScore";
import type { ProjectInfo, SearchHit } from "../../types";

/**
 * The palette's render model.
 *
 * This exists to make one class of bug unrepresentable. The palette
 * used to render rows grouped by category while resolving the
 * keyboard cursor against a differently-ordered array, so Enter on
 * "Open Projects" ran "Sign Desktop out". Rather than keep two
 * orderings in sync, there is now exactly one list: `rows` is what
 * gets rendered, and `selectable` is the same list with the
 * non-interactive headings removed. A row's index is assigned where
 * the row is created, so `selectable[i]` is by construction the i-th
 * row the user can see.
 */
export type SelectableRow =
  | { kind: "action"; key: string; index: number; action: PaletteAction }
  | { kind: "project"; key: string; index: number; project: ProjectInfo }
  | { kind: "session"; key: string; index: number; hit: SearchHit };

export type PaletteRow =
  | { kind: "heading"; key: string; label: string; loading?: boolean }
  | { kind: "empty"; key: string; text: string }
  | SelectableRow;

export interface PaletteRowModel {
  rows: PaletteRow[];
  selectable: SelectableRow[];
}

export function buildPaletteRows(args: {
  /** Already in display order — see `usePaletteActions.filter`. */
  actions: readonly PaletteAction[];
  projects: readonly ProjectInfo[];
  projectsLoading: boolean;
  sessions: readonly SearchHit[];
  sessionsLoading: boolean;
  query: string;
}): PaletteRowModel {
  const rows: PaletteRow[] = [];
  const selectable: SelectableRow[] = [];

  const push = (row: SelectableRow) => {
    rows.push(row);
    selectable.push(row);
  };

  for (const category of CATEGORY_ORDER) {
    const inCategory = args.actions.filter((a) => a.category === category);
    if (inCategory.length === 0) continue;
    rows.push({
      kind: "heading",
      key: `h-${category}`,
      label: PALETTE_CATEGORY_LABELS[category],
    });
    for (const action of inCategory) {
      push({
        kind: "action",
        key: `a-${action.id}`,
        index: selectable.length,
        action,
      });
    }
  }

  const searching = args.query.trim().length >= MIN_SEARCH_QUERY;

  /**
   * Search-backed groups render their heading only when they have
   * something to show or are still loading. A bare heading over an
   * empty group reads as a broken pane; the whole-palette empty state
   * below covers "nothing matched anywhere".
   */
  const pushSearchGroup = <T>(
    label: string,
    loading: boolean,
    items: readonly T[],
    toRow: (item: T, i: number, index: number) => SelectableRow,
  ) => {
    if (!searching || (!loading && items.length === 0)) return;
    rows.push({ kind: "heading", key: `h-${label}`, label, loading });
    items.forEach((item, i) => push(toRow(item, i, selectable.length)));
  };

  pushSearchGroup("Projects", args.projectsLoading, args.projects,
    (project, _i, index) => ({
      kind: "project",
      key: `p-${project.original_path}`,
      index,
      project,
    }));

  pushSearchGroup("Sessions", args.sessionsLoading, args.sessions,
    (hit, i, index) => ({
      kind: "session",
      // file_path repeats across hits from the same transcript, so
      // the ordinal is part of the key.
      key: `s-${hit.file_path}-${i}`,
      index,
      hit,
    }));

  if (selectable.length === 0 && !args.projectsLoading && !args.sessionsLoading) {
    rows.push({ kind: "empty", key: "empty", text: "No matches" });
  }

  return { rows, selectable };
}
