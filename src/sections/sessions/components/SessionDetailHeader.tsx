import { useRef, useState } from "react";
import { Button } from "../../../components/primitives/Button";
import { IconButton } from "../../../components/primitives/IconButton";
import {
  ContextMenu,
  type ContextMenuItem,
} from "../../../components/ContextMenu";
import { NF } from "../../../icons";
import type { SessionChunk, SessionRow } from "../../../types";
import { sessionCostEstimate, usePriceTable } from "../../../costs";
import { deriveSessionTitle } from "../format";
import { exportSession } from "../sessionExport";
import { maybeRedact } from "../../../lib/redactSecrets";
import { SessionDetailHeaderFull } from "./SessionDetailHeaderFull";
import { SessionDetailHeaderCompact } from "./SessionDetailHeaderCompact";

/**
 * Orchestrator for the session detail header. Owns the kebab menu
 * popover state, builds the menu item list, and picks between the
 * full and compact layouts based on `compact` (driven by the
 * transcript scroll position in the parent).
 *
 * Two non-negotiable invariants for the inline action footer:
 * Reveal is always visible (most-used verb during reading), and
 * everything else lives in the kebab. Keeps the action row to two
 * controls so it never wraps and never crowds the metadata.
 */
export function SessionDetailHeader({
  row,
  chunks,
  viewMode,
  contextOpen,
  compact,
  onBack,
  onReveal,
  onCopyFirstPrompt,
  onMoveClick,
  onToggleViewMode,
  onToggleContext,
  onError,
}: {
  row: SessionRow;
  /** Null when the chunked view is unavailable (older Tauri binary).
   * Drives whether the Raw/Chunked toggle item appears in the
   * kebab. */
  chunks: SessionChunk[] | null;
  viewMode: "chunks" | "raw";
  contextOpen: boolean;
  /** Parent toggles this when the transcript scrolls past a small
   * threshold. Switches us into the single-row layout. */
  compact: boolean;
  onBack?: () => void;
  onReveal: () => void;
  onCopyFirstPrompt: () => void;
  onMoveClick: () => void;
  onToggleViewMode: () => void;
  onToggleContext: () => void;
  /** Optional error sink for the export pipeline. */
  onError?: (message: string) => void;
}) {
  // Redact the first prompt before deriving the title so an
  // `sk-ant-…` token in the user's first message can't surface in the
  // window header. The redactor is idempotent, so passing it through
  // again on the tooltip path is safe and cheap.
  const safeFirstPrompt = maybeRedact(row.first_user_prompt);
  const cleanTitle = deriveSessionTitle(safeFirstPrompt);
  const title =
    cleanTitle ??
    (row.is_sidechain ? "Agent subsession" : "(untitled session)");

  // Price table is fetched once at this orchestrator level and
  // shared with the full layout via prop. Rendering it inside the
  // full layout would re-issue `pricingGet` every time the user
  // scrolls back to the top and re-mounts the full view.
  const { table: priceTable } = usePriceTable();
  const costUsd = sessionCostEstimate(priceTable, row.models, {
    input: row.tokens.input,
    output: row.tokens.output,
    cache_read: row.tokens.cache_read,
    cache_creation: row.tokens.cache_creation,
  });

  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);

  const menuItems: ContextMenuItem[] = [
    {
      label: "Move to project…",
      disabled: !row.project_from_transcript,
      disabledReason: row.project_from_transcript
        ? undefined
        : "no cwd recorded",
      onClick: onMoveClick,
    },
    ...(row.first_user_prompt
      ? [{ label: "Copy first prompt", onClick: onCopyFirstPrompt }]
      : []),
    { separator: true, label: "", onClick: () => {} },
    {
      label: "Export as Markdown",
      onClick: () => {
        void exportSession(row.file_path, "md", onError);
      },
    },
    {
      label: "Export as JSON",
      onClick: () => {
        void exportSession(row.file_path, "json", onError);
      },
    },
    { separator: true, label: "", onClick: () => {} },
    ...(chunks !== null
      ? [
          {
            label: viewMode === "chunks" ? "Raw events" : "Chunked view",
            onClick: onToggleViewMode,
          },
        ]
      : []),
    {
      label: contextOpen ? "Hide context" : "Show context",
      onClick: onToggleContext,
    },
  ];

  // Anchor against the kebab itself rather than `document.activeElement`.
  // `activeElement` is correct when the user clicked the kebab, but
  // it goes stale if focus is somewhere else (e.g. the search input
  // when the menu opens via a future shortcut). A direct ref reads the
  // intended button regardless of focus state.
  const kebabRef = useRef<HTMLButtonElement | null>(null);
  const openMenu = () => {
    const rect = kebabRef.current?.getBoundingClientRect();
    setMenu({
      x: rect ? rect.right : 0,
      y: rect ? rect.bottom : 0,
    });
  };

  const kebabNode = (
    <IconButton
      ref={kebabRef}
      glyph={NF.ellipsis}
      size="sm"
      onClick={openMenu}
      title="More actions"
      aria-label="More session actions"
      aria-haspopup="menu"
      aria-expanded={menu !== null}
    />
  );

  const revealNode = (
    <Button
      variant="ghost"
      glyph={NF.folderOpen}
      glyphColor="var(--fg-muted)"
      onClick={onReveal}
    >
      Reveal
    </Button>
  );

  const layout = compact ? (
    <SessionDetailHeaderCompact
      row={row}
      title={title}
      onBack={onBack}
      revealNode={revealNode}
      kebabNode={kebabNode}
    />
  ) : (
    <SessionDetailHeaderFull
      row={row}
      title={title}
      costUsd={costUsd}
      onBack={onBack}
      revealNode={revealNode}
      kebabNode={kebabNode}
    />
  );

  return (
    <>
      {layout}
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems}
          onClose={() => setMenu(null)}
        />
      )}
    </>
  );
}
