import { useState } from "react";
import { BackAffordance } from "../../../components/primitives/BackAffordance";
import { Button } from "../../../components/primitives/Button";
import { CopyButton } from "../../../components/CopyButton";
import { Glyph } from "../../../components/primitives/Glyph";
import { IconButton } from "../../../components/primitives/IconButton";
import { Tag } from "../../../components/primitives/Tag";
import { ContextMenu, type ContextMenuItem } from "../../../components/ContextMenu";
import { NF } from "../../../icons";
import type { SessionChunk, SessionRow } from "../../../types";
import {
  formatUsd,
  sessionCostEstimate,
  usePriceTable,
} from "../../../costs";
import { formatRelativeTime, formatSize } from "../../projects/format";
import {
  bestTimestampMs,
  deriveSessionTitle,
  formatTokens,
  modelBadge,
  projectBasename,
  shortSessionId,
} from "../format";
import { exportSession } from "../sessionExport";

/**
 * Top header strip of the session detail viewer. Two layouts:
 *
 *   - **Full** (transcript at the top): breadcrumb / two-line title /
 *     tag row / metadata row / Reveal + kebab.
 *   - **Compact** (user has scrolled down): single ~40px row with
 *     breadcrumb · title (1 line) · model · Reveal + kebab.
 *
 * The compact form is the load-bearing one for users on short
 * windows — once the reader is reading, the header gives back its
 * vertical real estate. Snap rather than animate, with a small
 * hysteresis (managed in the parent) so the boundary doesn't
 * flicker.
 *
 * Lifted out of `SessionDetail.tsx` per the project's one-component-
 * per-file rule. Pure presentational — every dependency comes in
 * through props; the parent owns lifecycle, network calls, and the
 * Move modal.
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
   * Drives whether the Raw/Chunked toggle button is rendered. */
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
  const lastTs = bestTimestampMs(row.last_ts, row.last_modified_ms);
  const firstTs = row.first_ts ? Date.parse(row.first_ts) : null;
  const project = projectBasename(row.project_path) || row.slug;
  const cleanTitle = deriveSessionTitle(row.first_user_prompt);
  const titleText =
    cleanTitle ?? (row.is_sidechain ? "Agent subsession" : "(untitled session)");
  // API-equivalent cost for this session. The subscription user's
  // "I'm saving $X" payoff — framed as a neutral hypothetical so we
  // don't claim savings without knowing the user's plan.
  const { table: priceTable } = usePriceTable();
  const costUsd = sessionCostEstimate(priceTable, row.models, {
    input: row.tokens.input,
    output: row.tokens.output,
    cache_read: row.tokens.cache_read,
    cache_creation: row.tokens.cache_creation,
  });

  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const openMenuFromTarget = (target: HTMLElement | null) => {
    const rect = target?.getBoundingClientRect();
    setMenu({
      x: rect ? rect.right - 0 : 0,
      y: rect ? rect.bottom + 4 : 0,
    });
  };

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
      ? [
          {
            label: "Copy first prompt",
            onClick: onCopyFirstPrompt,
          },
        ]
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

  const kebabButton = (
    <IconButton
      glyph={NF.ellipsis}
      size="sm"
      onClick={() => {
        const el = document.activeElement as HTMLElement | null;
        openMenuFromTarget(el);
      }}
      title="More actions"
      aria-label="More session actions"
      aria-haspopup="menu"
      aria-expanded={menu !== null}
    />
  );

  const revealButton = (
    <Button
      variant="ghost"
      glyph={NF.folderOpen}
      glyphColor="var(--fg-muted)"
      onClick={onReveal}
    >
      Reveal
    </Button>
  );

  if (compact) {
    return (
      <div
        style={{
          padding: "var(--sp-8) var(--sp-28)",
          borderBottom: "var(--bw-hair) solid var(--line)",
          flexShrink: 0,
          background: "var(--bg)",
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-12)",
          minHeight: "var(--sp-40)",
        }}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--sp-6)",
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            letterSpacing: "var(--ls-wide)",
            textTransform: "uppercase",
            flexShrink: 0,
          }}
        >
          {onBack ? (
            <BackAffordance
              label={project}
              onClick={onBack}
              title={`Back to session list for ${project}`}
            />
          ) : (
            <span>{project}</span>
          )}
          <Glyph g={NF.chevronR} style={{ fontSize: "var(--fs-3xs)" }} />
          <span className="mono" title={row.session_id}>
            {shortSessionId(row.session_id)}
          </span>
        </div>

        <h3
          style={{
            flex: 1,
            margin: 0,
            fontSize: "var(--fs-sm)",
            fontWeight: 500,
            color: "var(--fg)",
            letterSpacing: "var(--ls-normal)",
            textTransform: "none",
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
            minWidth: 0,
          }}
          title={row.first_user_prompt ?? titleText}
        >
          {titleText}
        </h3>

        <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-6)" }}>
          {row.has_error && (
            <Tag tone="warn" glyph={NF.warn}>
              error
            </Tag>
          )}
          {row.models.length > 0 && (
            <Tag tone="accent" title={row.models.join(", ")}>
              {modelBadge(row.models)}
            </Tag>
          )}
          {revealButton}
          {kebabButton}
        </div>

        {menu && (
          <ContextMenu
            x={menu.x}
            y={menu.y}
            items={menuItems}
            onClose={() => setMenu(null)}
          />
        )}
      </div>
    );
  }

  return (
    <div
      style={{
        padding: "var(--sp-20) var(--sp-28) var(--sp-14)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        flexShrink: 0,
        background: "var(--bg)",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
          marginBottom: "var(--sp-6)",
        }}
      >
        <div
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            letterSpacing: "var(--ls-wide)",
            textTransform: "uppercase",
            display: "flex",
            alignItems: "center",
            gap: "var(--sp-6)",
          }}
        >
          {onBack ? (
            <BackAffordance
              label={project}
              onClick={onBack}
              title={`Back to session list for ${project}`}
            />
          ) : (
            <span>{project}</span>
          )}
          <Glyph g={NF.chevronR} style={{ fontSize: "var(--fs-3xs)" }} />
          <span className="mono" title={row.session_id}>
            {shortSessionId(row.session_id)}
          </span>
          <CopyButton text={row.session_id} />
        </div>
      </div>

      <h3
        style={{
          margin: 0,
          fontSize: "var(--fs-md-lg)",
          fontWeight: 600,
          color: "var(--fg)",
          letterSpacing: "var(--ls-normal)",
          textTransform: "none",
          overflow: "hidden",
          textOverflow: "ellipsis",
          display: "-webkit-box",
          WebkitLineClamp: 2,
          WebkitBoxOrient: "vertical",
        }}
        title={row.first_user_prompt ?? undefined}
      >
        {titleText}
      </h3>

      <div
        style={{
          marginTop: "var(--sp-10)",
          display: "flex",
          flexWrap: "wrap",
          gap: "var(--sp-8)",
        }}
      >
        {row.has_error && (
          <Tag tone="warn" glyph={NF.warn}>
            error
          </Tag>
        )}
        {row.is_sidechain && <Tag tone="ghost">agent</Tag>}
        {row.models.length > 0 && (
          <Tag tone="accent" title={row.models.join(", ")}>
            {modelBadge(row.models)}
          </Tag>
        )}
        {row.git_branch && (
          <Tag tone="neutral" glyph={NF.branch}>
            {row.git_branch}
          </Tag>
        )}
        {row.cc_version && <Tag tone="ghost">cc {row.cc_version}</Tag>}
        {row.tokens.total > 0 && (
          <Tag
            tone="neutral"
            title={`input ${row.tokens.input} · output ${row.tokens.output} · cache r/w ${row.tokens.cache_read}/${row.tokens.cache_creation}`}
          >
            {formatTokens(row.tokens.total)} tok
          </Tag>
        )}
        {costUsd !== null && costUsd > 0 && (
          <Tag
            tone="neutral"
            title={`On pay-per-call API: ${formatUsd(costUsd)}. Subscription users don't pay this — it's what the same tokens would have cost at Anthropic's standard API rates.`}
          >
            {formatUsd(costUsd)} on API
          </Tag>
        )}
        {row.message_count > 0 && (
          <Tag tone="neutral">
            {row.message_count} turn{row.message_count === 1 ? "" : "s"}
          </Tag>
        )}
        <Tag tone="ghost">{formatSize(row.file_size_bytes)}</Tag>
      </div>

      <div
        style={{
          marginTop: "var(--sp-10)",
          display: "flex",
          flexWrap: "wrap",
          gap: "var(--sp-12) var(--sp-16)",
          alignItems: "center",
          color: "var(--fg-muted)",
          fontSize: "var(--fs-xs)",
        }}
      >
        <span
          title={row.project_path}
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: "var(--sp-6)",
            maxWidth: "100%",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          <Glyph g={NF.folder} style={{ fontSize: "var(--fs-2xs)" }} />
          <span
            className="mono"
            style={{
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {row.project_path}
          </span>
          <CopyButton text={row.project_path} />
        </span>
        {firstTs != null && (
          <span title={row.first_ts ?? ""}>
            Started {formatRelativeTime(firstTs)}
          </span>
        )}
        {lastTs != null && (
          <span title={row.last_ts ?? ""}>
            Last event {formatRelativeTime(lastTs)}
          </span>
        )}
      </div>

      <div
        style={{
          marginTop: "var(--sp-14)",
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
        }}
      >
        {revealButton}
        {kebabButton}
      </div>

      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems}
          onClose={() => setMenu(null)}
        />
      )}
    </div>
  );
}
