// Inline style objects shared by Settings → Relay's section components.
// Token references only; the values live in `styles/tokens.css`.

export const NOTE = {
  fontSize: "var(--fs-sm)",
  color: "var(--fg-muted)",
  lineHeight: "var(--lh-body)",
  marginTop: "var(--sp-6)",
} as const;
export const FAINT = { ...NOTE, color: "var(--fg-faint)" } as const;
export const WARN = { ...NOTE, color: "var(--warn)" } as const;
export const DANGER = { ...NOTE, color: "var(--danger)" } as const;
export const PANE = {
  display: "flex",
  flexDirection: "column",
  gap: "var(--sp-24)",
  maxWidth: "var(--content-cap-md)",
} as const;
export const SECTION = { display: "flex", flexDirection: "column" } as const;
export const ROW_NAME = { fontSize: "var(--fs-sm)" } as const;
export const ROW_META = { color: "var(--fg-faint)" } as const;
