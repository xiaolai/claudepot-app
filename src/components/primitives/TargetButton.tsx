import { useRef, useState } from "react";
import { NF, type NfIcon } from "../../icons";
import { Glyph } from "./Glyph";
import { ContextMenu, type ContextMenuItem } from "../ContextMenu";

export type TargetButtonState = "active" | "available" | "adopt" | "disabled";

interface TargetButtonProps {
  icon: NfIcon;
  label: string;
  state: TargetButtonState;
  /** Body click. Omitted (or undefined) leaves the body inert — used
   *  for `active` and `disabled` states where the verb is already
   *  realized or unavailable. */
  onPrimary?: () => void;
  /** Body tooltip / `title` attribute. */
  primaryTitle?: string;
  /** Secondary actions. Empty or missing → chevron is hidden, making
   *  the button a plain inline action (e.g. the `adopt` state). */
  menu?: ContextMenuItem[];
  /** Override the default aria-label. */
  "aria-label"?: string;
  /** Short reason rendered as a caption underneath the button when
   *  the state is `disabled`. Honors design.md: disabled buttons
   *  state their reason inline, not via a tooltip. Keep it under
   *  ~24 characters — longer copy belongs in an AnomalyBanner. */
  disabledReason?: string;
}

/**
 * Split button: body (primary verb) + chevron (popover menu). Used on
 * AccountCard to encode CLI / Desktop activeness *and* surface the
 * binding verbs inline — the chip ↔ context-menu split is gone.
 *
 * States:
 *   active     filled accent. Body disabled, chevron live.
 *   available  ghost outline. Body fires onPrimary, chevron lists
 *              secondary verbs.
 *   adopt      dashed outline. Body fires onPrimary (typically "bind
 *              current Desktop session"). No chevron.
 *   disabled   muted fill. Body inert; chevron is still live if the
 *              caller supplied a `menu` (e.g. CLI "Re-login…" when
 *              the token expired). To make the chevron inert too,
 *              pass an empty or missing `menu`. The reason for the
 *              disabled primary lives in the adjacent AnomalyBanner
 *              rather than next to the button.
 *
 * The menu reuses `ContextMenu` for keyboard nav and outside-click
 * dismissal. Anchored below the chevron's right edge.
 */
export function TargetButton({
  icon,
  label,
  state,
  onPrimary,
  primaryTitle,
  menu,
  "aria-label": ariaLabel,
  disabledReason,
}: TargetButtonProps) {
  const chevronRef = useRef<HTMLButtonElement>(null);
  const [menuPos, setMenuPos] = useState<{ x: number; y: number } | null>(null);

  const paint = paintFor(state);
  const hasMenu = !!menu && menu.length > 0;
  const bodyInert = state === "active" || state === "disabled" || !onPrimary;
  const showCaption = state === "disabled" && !!disabledReason;

  const toggleMenu = () => {
    if (menuPos) {
      setMenuPos(null);
      return;
    }
    const el = chevronRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    // ContextMenu clamps itself against the right/bottom viewport
    // edges, but not the left — anchor at `rect.right - MENU_W` so
    // the menu aligns to the chevron's right side, then clamp x to
    // a viewport-padding floor so a card near the window's left
    // edge doesn't kick the menu offscreen.
    const MENU_W = 200;
    const PADDING = 8;
    const x = Math.max(PADDING, rect.right - MENU_W);
    setMenuPos({ x, y: rect.bottom + 4 });
  };

  return (
    <>
      <span
        style={{
          display: "inline-flex",
          flexDirection: "column",
          alignItems: "stretch",
          gap: "var(--sp-2)",
        }}
      >
      <span
        style={{
          display: "inline-flex",
          alignItems: "center",
          height: "var(--btn-h-sm)",
          border: paint.border,
          background: paint.bg,
          color: paint.color,
          borderRadius: "var(--r-2)",
          overflow: "hidden",
        }}
      >
        <button
          type="button"
          disabled={bodyInert}
          onClick={bodyInert ? undefined : onPrimary}
          title={primaryTitle}
          aria-label={
            ariaLabel ?? `${label}${state === "active" ? " (active)" : ""}`
          }
          aria-pressed={state === "active"}
          className="pm-focus"
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: "var(--sp-6)",
            height: "100%",
            padding: "0 var(--sp-8)",
            background: "transparent",
            color: "inherit",
            border: "none",
            fontFamily: "inherit",
            fontSize: "var(--fs-xs)",
            fontWeight: 500,
            letterSpacing: "var(--ls-wide)",
            textTransform: "uppercase",
            cursor: bodyInert ? "default" : "pointer",
            opacity: state === "disabled" ? "var(--opacity-dimmed)" : 1,
          }}
        >
          <Glyph g={icon} style={{ fontSize: "var(--fs-xs)" }} />
          <span>{label}</span>
          {state === "active" && (
            <Glyph g={NF.check} style={{ fontSize: "var(--fs-xs)" }} />
          )}
        </button>
        {hasMenu && (
          <>
            <span
              aria-hidden
              style={{
                width: "var(--bw-hair)",
                alignSelf: "stretch",
                background: paint.divider,
              }}
            />
            <button
              type="button"
              ref={chevronRef}
              // Stop mousedown from reaching ContextMenu's document-level
              // outside-click handler. Without this, clicking the
              // chevron to close an open menu would first trigger
              // onClose (via mousedown) then onClick would reopen it.
              onMouseDown={(e) => e.stopPropagation()}
              onClick={toggleMenu}
              title={`${label} options`}
              aria-label={`${label} options`}
              aria-haspopup="menu"
              aria-expanded={menuPos !== null}
              className="pm-focus"
              style={{
                display: "inline-flex",
                alignItems: "center",
                justifyContent: "center",
                width: "var(--sp-20)",
                height: "100%",
                background: "transparent",
                color: "inherit",
                border: "none",
                cursor: "pointer",
              }}
            >
              <Glyph g={NF.chevronD} style={{ fontSize: "var(--fs-xs)" }} />
            </button>
          </>
        )}
      </span>
      {showCaption && (
        <span
          role="note"
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            textAlign: "center",
            letterSpacing: "0.02em",
            lineHeight: "var(--lh-tight)",
          }}
        >
          {disabledReason}
        </span>
      )}
      </span>
      {menuPos && menu && (
        <ContextMenu
          x={menuPos.x}
          y={menuPos.y}
          items={menu}
          onClose={() => setMenuPos(null)}
        />
      )}
    </>
  );
}

function paintFor(state: TargetButtonState): {
  bg: string;
  color: string;
  border: string;
  divider: string;
} {
  switch (state) {
    case "active":
      return {
        bg: "var(--accent)",
        color: "var(--on-color)",
        border: "var(--bw-hair) solid var(--accent)",
        divider: "color-mix(in oklch, var(--on-color) 30%, transparent)",
      };
    case "adopt":
      return {
        bg: "transparent",
        color: "var(--fg)",
        border: "var(--bw-hair) dashed var(--line-strong)",
        divider: "var(--line)",
      };
    case "disabled":
      return {
        bg: "var(--bg-sunken)",
        color: "var(--fg-faint)",
        border: "var(--bw-hair) solid var(--line)",
        divider: "var(--line)",
      };
    case "available":
    default:
      return {
        bg: "transparent",
        color: "var(--fg)",
        border: "var(--bw-hair) solid var(--line-strong)",
        divider: "var(--line)",
      };
  }
}
