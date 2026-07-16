// Shared vocabulary for the Knowledge surfaces.
//
// The Dashboard's stat cards and the Know view's per-project headers must
// speak the same visual language — the same tone for "enforced", the same
// bar for a project's trust mix — or the reframe fragments into two
// dialects. Extracted here so both import one source of truth (the doc's
// "extract Gazette's StatCard + toneColor" note).

import type { LessonCounts } from "../../api/sharedMemory";

// ─── tone → color ────────────────────────────────────────────────

/** The knowledge-state palette. Each tone pairs with a text badge or
 *  label at every call site — color never carries meaning alone
 *  (design.md accessibility floor). */
export type Tone = "accent" | "good" | "warn" | "neutral";

export function toneColor(tone: Tone): string {
  switch (tone) {
    case "accent":
      return "var(--accent)";
    case "good":
      return "var(--ok)";
    case "warn":
      return "var(--warn)";
    default:
      return "var(--fg-muted)";
  }
}

// ─── stat card ───────────────────────────────────────────────────

export interface StatCardProps {
  label: string;
  value: number | string;
  tone: Tone;
  hint: string;
  /** Optional emphasis: the headline signal gets a heavier value. */
  emphasis?: boolean;
  /** When set, the card becomes a button that routes to the next action
   *  (design.md: a headline number must click to something that changes
   *  it). A disabled reason is shown inline instead of a tooltip. */
  onClick?: () => void;
}

/** One dashboard cell: a big toned number, a label, and a one-line hint
 *  that says what the number *means* — never a bare count. A `value` of
 *  `"—"` is the honest placeholder for "not loaded / failed", distinct
 *  from a real `0`. */
export function StatCard({ label, value, tone, hint, emphasis, onClick }: StatCardProps) {
  const inner = (
    <>
      <span
        style={{
          fontSize: emphasis ? "var(--fs-2xl)" : "var(--fs-xl)",
          fontWeight: 600,
          lineHeight: 1.1,
          color: toneColor(tone),
        }}
      >
        {value}
      </span>
      <span style={{ fontSize: "var(--fs-base)", fontWeight: 500 }}>{label}</span>
      <span style={{ fontSize: "var(--fs-xs)", color: "var(--fg-muted)" }}>
        {hint}
      </span>
    </>
  );
  const boxStyle: React.CSSProperties = {
    border: "var(--sp-px) solid var(--line)",
    borderRadius: "var(--r-3)",
    padding: "var(--sp-16)",
    display: "flex",
    flexDirection: "column",
    gap: "var(--sp-4)",
    background: "var(--bg-raised)",
    textAlign: "left",
  };
  if (onClick) {
    return (
      <button
        type="button"
        className="pm-focus"
        onClick={onClick}
        style={{ ...boxStyle, cursor: "pointer", font: "inherit", color: "var(--fg)" }}
      >
        {inner}
      </button>
    );
  }
  return <div style={boxStyle}>{inner}</div>;
}

// ─── trust bar ───────────────────────────────────────────────────

/** The four buckets a project's curated knowledge falls into, in the
 *  order they stack left-to-right in the bar. `rejected` is deliberately
 *  absent — a settled "no" is not part of the trust picture. */
export interface TrustMix {
  /** Accepted AND compiled into a guard — cannot silently rot. */
  enforced: number;
  /** Accepted, not yet compiled to a check. */
  documented: number;
  /** Was accepted; the code it relied on moved. Back in the queue. */
  suspect: number;
  /** Awaiting your yes / no. */
  proposed: number;
}

export function trustMix(counts: LessonCounts): TrustMix {
  return {
    enforced: counts.enforced,
    documented: Math.max(0, counts.accepted - counts.enforced),
    suspect: counts.suspect,
    proposed: counts.proposed,
  };
}

export function trustTotal(mix: TrustMix): number {
  return mix.enforced + mix.documented + mix.suspect + mix.proposed;
}

const TRUST_SEGMENTS: { key: keyof TrustMix; label: string; color: string }[] = [
  { key: "enforced", label: "enforced", color: "var(--ok)" },
  { key: "documented", label: "documented", color: "var(--info)" },
  { key: "suspect", label: "suspect", color: "var(--warn)" },
  { key: "proposed", label: "proposed", color: "var(--accent)" },
];

/** A single horizontal bar showing a project's (or the roll-up's)
 *  enforced / documented / suspect / proposed proportions. Accessible:
 *  the bar is `role="img"` with an aria-label spelling out the counts, so
 *  the color split never has to be read to understand it. */
export function TrustBar({
  mix,
  height = 8,
  showLegend = false,
}: {
  mix: TrustMix;
  height?: number;
  showLegend?: boolean;
}) {
  const total = trustTotal(mix);
  const present = TRUST_SEGMENTS.filter((s) => mix[s.key] > 0);
  const label =
    total === 0
      ? "No curated knowledge yet"
      : present.map((s) => `${mix[s.key]} ${s.label}`).join(", ");

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-6)" }}>
      <div
        role="img"
        aria-label={label}
        title={label}
        style={{
          display: "flex",
          height,
          borderRadius: "var(--r-pill)",
          overflow: "hidden",
          background: "var(--bg-sunken)",
          border: "var(--sp-px) solid var(--line)",
        }}
      >
        {total > 0 &&
          present.map((s) => (
            <div
              key={s.key}
              style={{
                flexGrow: mix[s.key],
                flexBasis: 0,
                background: s.color,
              }}
            />
          ))}
      </div>
      {showLegend && total > 0 && (
        <div
          style={{
            display: "flex",
            flexWrap: "wrap",
            gap: "var(--sp-12)",
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-muted)",
          }}
        >
          {present.map((s) => (
            <span
              key={s.key}
              style={{ display: "inline-flex", alignItems: "center", gap: "var(--sp-4)" }}
            >
              <span
                aria-hidden="true"
                style={{
                  width: 8,
                  height: 8,
                  borderRadius: "var(--r-pill)",
                  background: s.color,
                }}
              />
              {mix[s.key]} {s.label}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
