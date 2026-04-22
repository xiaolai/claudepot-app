import { useVirtualizer } from "@tanstack/react-virtual";
import type { MouseEvent } from "react";
import type { SessionRow } from "../../../types";
import { ESTIMATED_ROW_PX } from "../sessionsTable.shared";
import { SessionRowView } from "./SessionRowView";

/**
 * Common shape for both the plain and virtualized list renderers.
 * Selection lives in the parent (`SessionsTable`'s caller) so a path
 * stays selected across filter / sort / virtualization-threshold
 * crossings.
 */
export interface ListProps {
  shown: SessionRow[];
  selectedId: string | null;
  onSelect: (filePath: string) => void;
  onContextMenu?: (e: MouseEvent, s: SessionRow) => void;
  searchSnippets?: Map<string, string>;
}

/**
 * Straight `<ul>` render. Used below the virtualization threshold and
 * as the fallback when the virtualizer boundary catches an error.
 */
export function PlainList({
  shown,
  selectedId,
  onSelect,
  onContextMenu,
  searchSnippets,
}: ListProps) {
  return (
    <ul
      role="listbox"
      aria-label="Sessions"
      style={{ listStyle: "none", margin: 0, padding: 0 }}
    >
      {shown.map((s) => (
        <SessionRowView
          key={s.file_path}
          session={s}
          active={s.file_path === selectedId}
          onSelect={onSelect}
          onContextMenu={onContextMenu}
          snippet={searchSnippets?.get(s.file_path)}
        />
      ))}
    </ul>
  );
}

/**
 * Virtualized render. The `<ul>` sits inside the scroll container and
 * acts as the virtualizer's content element — its height is the total
 * virtual size, and each mounted `<li>` is absolutely positioned via
 * translateY. Listbox semantics survive because every child remains a
 * direct `<li role="option">` of the `<ul>`.
 */
export function VirtualList({
  shown,
  selectedId,
  onSelect,
  onContextMenu,
  searchSnippets,
  scrollEl,
}: ListProps & { scrollEl: HTMLDivElement | null }) {
  const rowVirtualizer = useVirtualizer({
    count: shown.length,
    getScrollElement: () => scrollEl,
    estimateSize: () => ESTIMATED_ROW_PX,
    overscan: 8,
    getItemKey: (index) => shown[index].file_path,
  });

  const items = rowVirtualizer.getVirtualItems();

  return (
    <ul
      role="listbox"
      aria-label="Sessions"
      style={{
        listStyle: "none",
        margin: 0,
        padding: 0,
        position: "relative",
        height: rowVirtualizer.getTotalSize(),
      }}
    >
      {items.map((virtualRow) => {
        const s = shown[virtualRow.index];
        return (
          <SessionRowView
            key={s.file_path}
            session={s}
            active={s.file_path === selectedId}
            onSelect={onSelect}
            onContextMenu={onContextMenu}
            snippet={searchSnippets?.get(s.file_path)}
            // Primitive props keep `React.memo`'s shallow equality
            // effective on the virtualized path. An inline style
            // object literal here would change identity every render
            // and defeat memo for every visible row.
            virtualStart={virtualRow.start}
            virtualIndex={virtualRow.index}
            virtualSetSize={shown.length}
            measureRef={rowVirtualizer.measureElement}
          />
        );
      })}
    </ul>
  );
}
