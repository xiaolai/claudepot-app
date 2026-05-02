// Activities → Cost → Top Prompts panel.
//
// Compact ranked list of the install's costliest prompts in the
// active window + tier. Renders only when the backend returned at
// least one row — fresh installs with no per-turn data yet show no
// panel at all (render-if-nonzero per design.md).
//
// Pure presentation: the panel doesn't fetch its own data because
// the parent (CostTab) already owns the loading state for the
// matching window selector. Pulling the fetch up keeps the two
// surfaces (cost table + top prompts) consistent across re-fetches.

import type { CostlyTurn, TopCostlyPrompts } from "../../types";
import { shortModelId } from "./CostTabHelpers";

export function TopPromptsPanel({ data }: { data: TopCostlyPrompts }) {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-6)",
      }}
    >
      <div
        style={{
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
          letterSpacing: "var(--ls-wide)",
          textTransform: "uppercase",
        }}
      >
        Top {data.turns.length} costly prompt{data.turns.length === 1 ? "" : "s"}
      </div>
      <ol
        style={{
          listStyle: "none",
          margin: 0,
          padding: 0,
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-4)",
        }}
      >
        {data.turns.map((t, i) => (
          <CostlyTurnRow key={`${t.file_path}:${t.turn_index}`} turn={t} rank={i + 1} />
        ))}
      </ol>
    </div>
  );
}

function CostlyTurnRow({ turn, rank }: { turn: CostlyTurn; rank: number }) {
  const project = displayPath(turn.project_path);
  const preview = turn.user_prompt_preview ?? "(no prompt recorded)";
  const totalTokens =
    turn.tokens_input + turn.tokens_output + turn.tokens_cache_creation + turn.tokens_cache_read;
  return (
    <li
      style={{
        display: "grid",
        gridTemplateColumns: "var(--rank-col) minmax(0, 1fr) auto auto",
        alignItems: "center",
        gap: "var(--sp-10)",
        padding: "var(--sp-6) var(--sp-8)",
        background: "var(--bg-raised)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-1)",
        // The rank-col custom prop lets us keep the rank column tight
        // without a magic number. 28px fits "10." at the current font.
        ["--rank-col" as keyof React.CSSProperties]: "tokens.sp[28]",
      } as React.CSSProperties}
    >
      <div
        style={{
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
          fontVariantNumeric: "tabular-nums",
          textAlign: "right",
        }}
      >
        {rank}.
      </div>
      <div
        style={{
          minWidth: 0,
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-1)",
        }}
      >
        <div
          title={preview}
          style={{
            fontSize: "var(--fs-xs)",
            color: "var(--fg)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {preview}
        </div>
        <div
          title={turn.project_path}
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {project} · turn {turn.turn_index + 1}
        </div>
      </div>
      <span
        title={`${turn.model} · ${formatCompact(totalTokens)} tokens`}
        style={{
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-muted)",
          background: "var(--bg-sunken)",
          border: "var(--bw-hair) solid var(--line)",
          borderRadius: "var(--r-1)",
          padding: "var(--sp-1) var(--sp-4)",
          whiteSpace: "nowrap",
        }}
      >
        {shortModelId(turn.model)}
      </span>
      <div
        style={{
          fontSize: "var(--fs-xs)",
          color: "var(--fg)",
          fontVariantNumeric: "tabular-nums",
          textAlign: "right",
          minWidth: "var(--cost-col, tokens.sp[60])",
        }}
      >
        ${turn.cost_usd.toFixed(2)}
      </div>
    </li>
  );
}

/** Render the project's basename — duplicated from CostTab to keep
 *  the panel self-contained. CC project CWDs share long
 *  `/Users/<user>/...` prefixes that waste space; the leaf folder
 *  is what users recognise. Windows-aware for `\` separators. */
function displayPath(p: string): string {
  if (!p) return p;
  const trimmed = p.replace(/[/\\]+$/, "");
  const segs = trimmed.split(/[/\\]/).filter(Boolean);
  return segs[segs.length - 1] ?? trimmed;
}

function formatCompact(n: number): string {
  if (n < 1_000) return String(n);
  if (n < 1_000_000) return `${(n / 1_000).toFixed(1)}k`;
  if (n < 1_000_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  return `${(n / 1_000_000_000).toFixed(2)}B`;
}
