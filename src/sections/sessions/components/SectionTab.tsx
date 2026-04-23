/**
 * Proper `role="tab"` button for the Sessions/Cleanup tab strip. The
 * paper-mono `FilterChip` primitive uses `role="switch"`, which
 * conflicts with `role="tablist"` parents — assistive tech announces
 * those as toggle switches instead of tabs and never associates them
 * with the panel they actually control. This thin button mirrors
 * FilterChip's styling but emits the correct ARIA contract:
 * `role="tab"`, `aria-selected`, and `aria-controls` linking to the
 * tabpanel.
 */
export function SectionTab({
  id,
  panelId,
  label,
  active,
  onSelect,
  indicator,
}: {
  id: string;
  panelId: string;
  label: string;
  active: boolean;
  onSelect: () => void;
  /** Optional trailing badge rendered inside the tab. Pass a string
   *  for a text indicator or `true` to draw a small filled dot. */
  indicator?: string | number | true;
}) {
  return (
    <button
      id={id}
      type="button"
      role="tab"
      aria-selected={active}
      aria-controls={panelId}
      tabIndex={active ? 0 : -1}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-6)",
        height: "var(--sp-24)",
        padding: "0 var(--sp-10)",
        fontSize: "var(--fs-xs)",
        fontWeight: 500,
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
        color: active ? "var(--accent-ink)" : "var(--fg-muted)",
        background: active ? "var(--accent-soft)" : "var(--bg-sunken)",
        border: `var(--bw-hair) solid ${active ? "var(--accent-border)" : "var(--line)"}`,
        borderRadius: "var(--r-1)",
        cursor: "pointer",
        whiteSpace: "nowrap",
        outlineOffset: 2,
      }}
    >
      {label}
      {indicator !== undefined && (
        <span
          aria-hidden
          style={
            indicator === true
              ? {
                  // Render-if-nonzero dot — 6px, filled accent when
                  // inactive (to draw the eye) and muted when active.
                  width: 6,
                  height: 6,
                  borderRadius: "50%",
                  background: active ? "var(--on-color)" : "var(--accent)",
                  flexShrink: 0,
                }
              : {
                  fontSize: "var(--fs-2xs)",
                  color: active ? "var(--on-color)" : "var(--fg-faint)",
                  fontVariantNumeric: "tabular-nums",
                }
          }
        >
          {indicator === true ? "" : indicator}
        </span>
      )}
    </button>
  );
}
