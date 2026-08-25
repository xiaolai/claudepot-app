import type { CSSProperties, ReactNode } from "react";

interface SectionLabelProps {
  children: ReactNode;
  /** Optional right-aligned content (a button, badge, etc). */
  right?: ReactNode;
  /**
   * Horizontal inset, for a container whose ROWS are themselves inset —
   * a sidebar strip, a padded list — where a flush label would hang
   * left of everything it labels. Pass the container's own gutter.
   *
   * Omit it in a content pane. The label then starts where the prose
   * starts, which is what a heading naming a block below it should do.
   *
   * The default used to be `var(--sp-14)`, which was wrong nearly
   * everywhere and was being corrected by hand: `KeysSection` twice,
   * `EnvVaultSection` once, and `RemotePane` would have been six more.
   * Measured against the running app, that default put every heading
   * 14px right of its own content in Retention (×4), MCP (×2),
   * Knowledge (×3) and Updates (×1) — and 6px off the rows in the one
   * place an inset was genuinely wanted, because the sidebar's gutter
   * is 8px, not 14.
   *
   * So the inset is not the label's to guess. A heading does not know
   * its container's gutter; the container does.
   */
  inset?: string;
  style?: CSSProperties;
}

/** Uppercase section divider label — "ACCOUNTS", "~/.claude", etc. */
export function SectionLabel({ children, right, inset, style }: SectionLabelProps) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        // Vertical rhythm is the label's own business; the horizontal
        // inset belongs to whatever contains it.
        padding: `var(--sp-12) ${inset ?? "0"} var(--sp-6)`,
        ...style,
      }}
    >
      <span className="mono-cap" style={{ color: "var(--fg-faint)" }}>
        {children}
      </span>
      {right}
    </div>
  );
}
