import React, { type CSSProperties, useState } from "react";
import { IconButton } from "../../../components/primitives/IconButton";
import { NF } from "../../../icons";
import { DETAIL_QUERY_MIN_LEN } from "../sessionDetail.search";

/**
 * Visual primitives shared by the two transcript renderers
 * (`SessionEventView` and `SessionChunkView`). Both used to inline
 * near-identical Bubble / Divider / Body / highlight / escapeRegex
 * helpers; extracting them here keeps the two views in lockstep and
 * removes the drift risk of maintaining parallel implementations.
 */

export type BubbleTone = "sunken" | "accent" | "faint" | "ghost";

const BUBBLE_PALETTE: Record<BubbleTone, { bg: string; bd: string }> = {
  sunken: { bg: "var(--bg-sunken)", bd: "var(--line)" },
  accent: { bg: "var(--accent-soft)", bd: "var(--accent-border)" },
  faint: { bg: "var(--bg-raised)", bd: "var(--line)" },
  ghost: { bg: "transparent", bd: "var(--line)" },
};

/**
 * Chat-style speech bubble. `side` chooses justification (user
 * messages on the right, assistant on the left); `tone` picks the
 * background+border palette; `mono` swaps to the JetBrainsMono face
 * for code-shaped content.
 */
export function Bubble({
  side,
  tone,
  mono,
  children,
}: {
  side: "left" | "right";
  tone: BubbleTone;
  mono?: boolean;
  children: React.ReactNode;
}) {
  const p = BUBBLE_PALETTE[tone];
  return (
    <div
      style={{
        display: "flex",
        justifyContent: side === "right" ? "flex-end" : "flex-start",
        marginBottom: "var(--sp-10)",
      }}
    >
      <div
        style={{
          maxWidth: "min(var(--content-cap-lg), 92%)",
          minWidth: "min(280px, 60%)",
          padding: "var(--sp-10) var(--sp-14)",
          background: p.bg,
          border: `var(--bw-hair) solid ${p.bd}`,
          borderRadius: "var(--r-2)",
          fontFamily: mono ? "var(--font)" : undefined,
          fontSize: "var(--fs-sm)",
          color: "var(--fg)",
          whiteSpace: "pre-wrap",
          wordBreak: "break-word",
        }}
      >
        {children}
      </div>
    </div>
  );
}

/**
 * Horizontal divider used between transcript phases / context
 * boundaries. Children render in the centre, between two thin lines.
 */
export function Divider({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-10)",
        margin: "var(--sp-14) 0",
      }}
    >
      <div style={{ flex: 1, height: "var(--sp-1)", background: "var(--line)" }} />
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-6)",
          color: "var(--fg-faint)",
          fontSize: "var(--fs-xs)",
        }}
      >
        {children}
      </div>
      <div style={{ flex: 1, height: "var(--sp-1)", background: "var(--line)" }} />
    </div>
  );
}

/**
 * Truncating body cell with a "Show N more chars / Collapse" toggle.
 * `clamp` is the visible-character cap; rendered on top of `highlight`
 * so the active search term is marked even inside the trimmed slice.
 */
export function Body({
  text,
  searchTerm,
  mono,
  clamp,
  tone,
}: {
  text: string;
  searchTerm: string;
  mono?: boolean;
  /** Visible-character cap. Each transcript view passes its own
   * constant — chunk view uses TEXT_CLAMP, event view uses
   * MESSAGE_CLAMP — so the two stay tunable independently. */
  clamp: number;
  tone?: "ghost" | "warn";
}) {
  const [expanded, setExpanded] = useState(false);
  const trimmed = text ?? "";
  const overflow = trimmed.length > clamp;
  const visible = expanded || !overflow ? trimmed : trimmed.slice(0, clamp);

  const baseStyle: CSSProperties = {
    fontFamily: mono ? "var(--font)" : undefined,
    fontSize: mono ? "var(--fs-xs)" : "var(--fs-sm)",
    color:
      tone === "warn"
        ? "var(--warn)"
        : tone === "ghost"
          ? "var(--fg-muted)"
          : "var(--fg)",
    whiteSpace: "pre-wrap",
    wordBreak: "break-word",
  };

  return (
    <>
      <div style={baseStyle}>{highlight(visible, searchTerm)}</div>
      {overflow && (
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          style={{
            marginTop: "var(--sp-4)",
            background: "transparent",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-1)",
            color: "var(--fg-muted)",
            fontSize: "var(--fs-xs)",
            padding: "var(--sp-2) var(--sp-8)",
            cursor: "pointer",
            letterSpacing: "var(--ls-wide)",
            textTransform: "uppercase",
          }}
        >
          {expanded
            ? "Collapse"
            : `Show ${trimmed.length - clamp} more chars`}
        </button>
      )}
    </>
  );
}

/**
 * Turn-level fold threshold, in characters.
 *
 * Distinct from `Body`'s `clamp`, which trims *within* one text block.
 * This one folds an entire turn — header stays, body collapses to a
 * one-line preview — so a transcript full of long answers stays
 * scannable instead of being a wall you scroll past.
 */
export const TURN_FOLD_CHARS = 1200;

/**
 * First *meaningful* line of `text`, ellipsised for the folded preview.
 *
 * Skips bare section markers. `SessionChunkView` builds an AI turn's
 * text by prefixing each thinking block with a `[thinking]` line, so a
 * turn that opens by thinking would otherwise preview as the literal
 * string "[thinking]" — a fold hint that tells the reader nothing. Skip
 * those and show the first real line of prose instead.
 */
function previewLine(text: string): string {
  const first =
    text
      .split("\n")
      .map((l) => l.trim())
      .find((l) => l.length > 0 && !/^\[[a-z][a-z ._-]*\]$/i.test(l)) ?? "";
  return first.length > 140 ? `${first.slice(0, 140)}…` : first;
}

const FOLD_PREVIEW_BUTTON: CSSProperties = {
  display: "block",
  width: "100%",
  textAlign: "left",
  background: "transparent",
  border: "none",
  padding: "var(--sp-4) 0 0",
  cursor: "pointer",
  font: "inherit",
};

const FOLD_PREVIEW_LINE: CSSProperties = {
  color: "var(--fg-muted)",
  fontSize: "var(--fs-sm)",
  whiteSpace: "nowrap",
  overflow: "hidden",
  textOverflow: "ellipsis",
};

const FOLD_PREVIEW_HINT: CSSProperties = {
  marginTop: "var(--sp-2)",
  color: "var(--fg-faint)",
  fontSize: "var(--fs-xs)",
  letterSpacing: "var(--ls-wide)",
  textTransform: "uppercase",
};

/** What a folded turn shows in place of its body: opening line + size. */
function FoldPreview({
  text,
  onExpand,
}: {
  text: string;
  onExpand: () => void;
}) {
  return (
    <button type="button" onClick={onExpand} style={FOLD_PREVIEW_BUTTON}>
      <div style={FOLD_PREVIEW_LINE}>{previewLine(text)}</div>
      <div style={FOLD_PREVIEW_HINT}>
        {text.split("\n").length} lines · {text.length.toLocaleString()} chars
      </div>
    </button>
  );
}

/**
 * A `Bubble` whose body folds when the turn is long.
 *
 * Fold rules:
 *  - A turn under `TURN_FOLD_CHARS` is never foldable — no chevron, no
 *    behavior change. Short turns render exactly as before.
 *  - A turn over the threshold starts **folded**, showing only its
 *    header plus a one-line preview and a size hint.
 *  - A turn that matches the live search is **never** folded. The chunk
 *    list is already filtered to matches, so folding one would hide the
 *    very hit the user searched for. This overrides both the default
 *    and an explicit user fold — search visibility wins.
 *  - An explicit toggle by the user otherwise sticks, and survives
 *    "Show older" paging because chunks keep stable React keys.
 */
export function FoldableBubble({
  side,
  tone,
  mono,
  header,
  foldText,
  searchTerm,
  children,
}: {
  side: "left" | "right";
  tone: BubbleTone;
  mono?: boolean;
  /** Title row (label, timestamp, actions). Always visible, folded or not. */
  header: React.ReactNode;
  /**
   * Plain text of the turn. Drives the fold decision, the preview line,
   * and the size hint. It need not cover everything `children` renders
   * — an AI turn's tool cards are deliberately excluded, since the
   * header already reports the tool count.
   */
  foldText: string;
  searchTerm: string;
  children: React.ReactNode;
}) {
  const foldable = foldText.length > TURN_FOLD_CHARS;
  // `null` = follow the default; a boolean = the user has decided.
  const [userFolded, setUserFolded] = useState<boolean | null>(null);

  // While a search is active, NOTHING folds.
  //
  // The transcript already filters chunks to those matching the query
  // (`chunkMatchesSearch`), so every turn still on screen contains a hit
  // — and that hit is not necessarily in `foldText`. `chunkMatchesSearch`
  // also matches tool inputs and tool results, which render inside
  // `children` and never appear in `foldText` (an AI turn's foldText is
  // its prose only). An earlier version folded unless `foldText` itself
  // contained the term; driving the real app showed 10 of 48 matching
  // turns staying folded on a tool-result hit — search hiding its own
  // result, the precise failure the fold must never cause.
  // Same floor `normalizeDetailQuery` applies, imported rather than
  // re-typed: if the two drift, the fold and the chunk filter disagree
  // about whether a search is running.
  const searchActive = searchTerm.trim().length >= DETAIL_QUERY_MIN_LEN;

  const folded = foldable && !searchActive && (userFolded ?? true);

  if (!foldable) {
    return (
      <Bubble side={side} tone={tone} mono={mono}>
        {header}
        {children}
      </Bubble>
    );
  }

  return (
    <Bubble side={side} tone={tone} mono={mono}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-6)",
        }}
      >
        <IconButton
          glyph={folded ? NF.chevronR : NF.chevronD}
          size="sm"
          onClick={() => setUserFolded(!folded)}
          title={folded ? "Expand turn" : "Collapse turn"}
          aria-label={folded ? "Expand turn" : "Collapse turn"}
          aria-expanded={!folded}
        />
        <div style={{ flex: 1, minWidth: 0 }}>{header}</div>
      </div>
      {folded ? (
        <FoldPreview text={foldText} onExpand={() => setUserFolded(false)} />
      ) : (
        children
      )}
    </Bubble>
  );
}

/**
 * Case-insensitive highlight of the search term. Chunks the source
 * string on the search match, wrapping hits in a `<mark>` with the
 * accent-soft background. Short-circuits on empty queries so we don't
 * pay the regex cost per event on every keystroke of the filter box.
 */
export function highlight(text: string, term: string): React.ReactNode {
  if (!term || term.length < 2) return text;
  try {
    const pattern = new RegExp(escapeRegex(term), "gi");
    const parts: React.ReactNode[] = [];
    let lastIdx = 0;
    let match: RegExpExecArray | null;
    let key = 0;
    while ((match = pattern.exec(text)) !== null) {
      if (match.index > lastIdx) parts.push(text.slice(lastIdx, match.index));
      parts.push(
        <mark
          key={`h${key++}`}
          style={{
            background: "var(--accent-soft)",
            color: "var(--accent-ink)",
          }}
        >
          {match[0]}
        </mark>,
      );
      lastIdx = match.index + match[0].length;
      if (match.index === pattern.lastIndex) pattern.lastIndex += 1;
    }
    if (lastIdx < text.length) parts.push(text.slice(lastIdx));
    return parts;
  } catch {
    return text;
  }
}

export function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
