import { useCallback, useEffect, useRef, useState } from "react";
import type { SelectableRow } from "./rows";

/**
 * Cursor + activation for the ⌘K palette.
 *
 * Extracted from `CommandPalette` so the component renders and this
 * owns "which row is current and what running it means". Keeping them
 * together made the palette a single unit doing search state, row
 * modeling, keyboard handling, activation dispatch and rendering.
 *
 * The invariant this module exists to hold: **the row reported as
 * selected is the row that runs.** `selectable` is the rendered rows
 * minus headings, indexed in visible order, so `selectable[i]` is the
 * i-th row on screen by construction — there is no second ordering to
 * fall out of sync with.
 */
export function usePaletteSelection(args: {
  selectable: readonly SelectableRow[];
  /** Reset the cursor whenever this changes — i.e. a new query. */
  resetKey: string;
  onClose: () => void;
  /** Run a row. Called by both Enter and pointer activation. */
  onRun: (row: SelectableRow) => void;
}): {
  selectedIndex: number;
  setSelectedIndex: (i: number) => void;
  activate: (row: SelectableRow) => void;
  handleKeyDown: (e: React.KeyboardEvent) => void;
  listRef: React.RefObject<HTMLUListElement | null>;
} {
  const { selectable, resetKey, onClose, onRun } = args;
  const [selectedIndex, setSelectedIndex] = useState(0);
  const listRef = useRef<HTMLUListElement>(null);

  // Snap back to the top whenever the result set changes, so the
  // cursor can't sit past the end of a shorter list.
  useEffect(() => {
    setSelectedIndex(0);
  }, [resetKey]);
  useEffect(() => {
    if (selectedIndex >= selectable.length) setSelectedIndex(0);
  }, [selectable.length, selectedIndex]);

  useEffect(() => {
    // Query `.palette-item` specifically — the list also contains
    // `.palette-group-label` dividers. Indexing against every child
    // would scroll a heading into view instead of the selected row.
    const el = listRef.current?.querySelectorAll<HTMLElement>(
      ".palette-item",
    )[selectedIndex];
    el?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  const activate = useCallback(
    (row: SelectableRow) => {
      // Move the cursor onto the row being run. Pointer activation is
      // already correct without this — you get what you clicked — but
      // a touch tap fires no mouseenter, so `aria-selected` could name
      // a different row at the moment of activation. Syncing keeps the
      // invariant literally true.
      setSelectedIndex(row.index);
      onRun(row);
      onClose();
    },
    [onRun, onClose],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      const last = selectable.length - 1;
      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          if (last >= 0) setSelectedIndex((i) => Math.min(i + 1, last));
          break;
        case "ArrowUp":
          e.preventDefault();
          setSelectedIndex((i) => Math.max(i - 1, 0));
          break;
        case "Home":
          e.preventDefault();
          setSelectedIndex(0);
          break;
        case "End":
          e.preventDefault();
          if (last >= 0) setSelectedIndex(last);
          break;
        case "Enter": {
          e.preventDefault();
          const row = selectable[selectedIndex];
          if (row) activate(row);
          break;
        }
        case "Escape":
          e.preventDefault();
          onClose();
          break;
      }
    },
    [selectable, selectedIndex, activate, onClose],
  );

  return { selectedIndex, setSelectedIndex, activate, handleKeyDown, listRef };
}
