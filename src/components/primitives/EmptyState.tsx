import type { ReactNode } from "react";
import { Glyph } from "./Glyph";
import type { NfIcon } from "../../icons";

/**
 * The one "there is nothing here" surface.
 *
 * Five components were called `EmptyState`, shared no code, and
 * disagreed about what an empty state is: some had a title and body,
 * some a CTA, some were a bare `<p>` with a translated string. Empty
 * states are the first thing a new user sees in most sections, so the
 * quality of first-run varied by whichever section they happened to
 * land on. `primitives/` had an `Avatar`, a `Tag`, a `Kbd` and a
 * `Skeleton` — but nowhere to put the fix.
 *
 * ## `action` is required
 *
 * Not optional-with-a-default. An empty state with no next action is
 * usually a missing feature rather than a missing string, and making
 * that a deliberate `action={null}` puts the decision where a reviewer
 * can see it. Three of the migrated sites turned out to genuinely have
 * no next action; two were missing one.
 *
 * ## Two variants, because there are two real shapes
 *
 * - `block` (default) — a section or pane with nothing in it. Padded,
 *   dashed enclosure, optionally centered.
 * - `inline` — a note *inside* an otherwise-populated pane ("no
 *   sections matched"). One line, no enclosure. Rendering these as
 *   block states was the reason `HealthPane` grew its own.
 *
 * Resisted a third variant for `ThirdPartySection`'s left-aligned
 * explainer: that is `block` with `align="start"`, and a variant per
 * call site is how the original five happened.
 */
export interface EmptyStateProps {
  /** Optional leading icon. Omit when the copy carries the meaning. */
  glyph?: NfIcon;
  /** Short heading. Omit for `inline`, where the body is the whole message. */
  title?: string;
  /** The message. `ReactNode` so callers can pass <Trans> output. */
  body: ReactNode;
  /**
   * The next step, or an explicit `null` when there genuinely is not
   * one. Required — see the note above.
   */
  action: ReactNode | null;
  variant?: "block" | "inline";
  /** `block` only. Left-align when the body is prose rather than a line. */
  align?: "center" | "start";
}

export function EmptyState({
  glyph,
  title,
  body,
  action,
  variant = "block",
  align = "center",
}: EmptyStateProps) {
  if (variant === "inline") {
    return (
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
          padding: "var(--sp-12) 0",
          color: "var(--fg-muted)",
          fontSize: "var(--fs-sm)",
        }}
      >
        {glyph && <Glyph g={glyph} color="var(--fg-faint)" />}
        <span>{body}</span>
        {action}
      </div>
    );
  }

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: align === "center" ? "center" : "flex-start",
        gap: "var(--sp-12)",
        padding: "var(--sp-32) var(--sp-16)",
        border: "var(--bw-hair) dashed var(--line)",
        borderRadius: "var(--r-3)",
        color: "var(--fg-muted)",
        textAlign: align === "center" ? "center" : "left",
      }}
    >
      {glyph && <Glyph g={glyph} color="var(--fg-faint)" />}
      {title && (
        <h3 style={{ margin: 0, fontSize: "var(--fs-md)", color: "var(--fg)" }}>
          {title}
        </h3>
      )}
      <div
        style={{
          margin: 0,
          fontSize: "var(--fs-sm)",
          maxWidth: "60ch",
          lineHeight: "var(--lh-loose)",
        }}
      >
        {body}
      </div>
      {action}
    </div>
  );
}
