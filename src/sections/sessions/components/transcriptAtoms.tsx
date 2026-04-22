import React, { type CSSProperties, useState } from "react";

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
      <div style={{ flex: 1, height: 1, background: "var(--line)" }} />
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
      <div style={{ flex: 1, height: 1, background: "var(--line)" }} />
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
