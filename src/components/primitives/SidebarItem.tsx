import { type ReactNode, useState } from "react";
import type { NfIcon } from "../../icons";
import { Glyph } from "./Glyph";

interface SidebarItemProps {
  glyph?: NfIcon;
  label: string;
  active?: boolean;
  badge?: ReactNode;
  onClick?: () => void;
  /** Tree indent level. 0 = top-level, 1 = one step in, etc. */
  indent?: number;
  /** Optional trailing widget (chevron, dot). */
  trailing?: ReactNode;
  title?: string;
  /**
   * Icon-only rendering for the rail-width collapsed sidebar. Hides
   * the label and trailing widget, centers the glyph, and renders a
   * numeric badge as a small dot instead of a number. The `title`
   * becomes the hover/screen-reader label (callers should pass it).
   */
  collapsed?: boolean;
}

/**
 * Single row in the left sidebar. Ghost at rest, hover fills with
 * `--bg-hover`, active fills with `--bg-active` + left border
 * (`--bw-strong`) in accent + weight 600.
 */
export function SidebarItem({
  glyph,
  label,
  active,
  badge,
  onClick,
  indent = 0,
  trailing,
  title,
  collapsed = false,
}: SidebarItemProps) {
  const [hover, setHover] = useState(false);
  const indentPad = indent > 0 ? `calc(var(--sp-10) + var(--sp-14) * ${indent})` : "var(--sp-10)";
  // In collapsed mode the row centers a single glyph at rail width.
  // A numeric badge collapses to a small dot — the count isn't legible
  // at this width, but presence-of-badge still is.
  const hasBadge = badge != null && badge !== false;
  return (
    <button
      type="button"
      title={collapsed ? (title ?? label) : title}
      // Collapsed: the label is hidden so screen-reader users need it
      // via aria-label. Surface the badge content too — the corner dot
      // is aria-hidden and the expanded-mode badge is read out, so AT
      // users would otherwise lose the signal entirely.
      //
      // For primitive badges (numbers, strings) we include the literal
      // value — `Accounts (3)` is honest. For arbitrary ReactNode
      // badges (a chip with a glyph, say) we fall back to a neutral
      // `(badge)` suffix rather than guessing semantics; phrasings
      // like `has updates` over-promise when the badge is just a count.
      aria-label={
        collapsed
          ? hasBadge
            ? typeof badge === "string" || typeof badge === "number"
              ? `${label} (${badge})`
              : `${label} (badge)`
            : label
          : undefined
      }
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      aria-current={active ? "page" : undefined}
      className="pm-focus"
      style={{
        width: "100%",
        display: "flex",
        alignItems: "center",
        justifyContent: collapsed ? "center" : undefined,
        gap: "var(--sp-10)",
        height: "var(--row-height)",
        padding: collapsed ? 0 : `0 var(--sp-10) 0 ${indentPad}`,
        margin: "var(--sp-px) 0",
        fontSize: "var(--fs-sm)",
        fontWeight: active ? 600 : 500,
        color: active
          ? "var(--fg)"
          : hover
            ? "var(--fg)"
            : "var(--fg-muted)",
        background: active
          ? "var(--bg-active)"
          : hover
            ? "var(--bg-hover)"
            : "transparent",
        borderRadius: "var(--r-2)",
        borderLeft: active
          ? "var(--bw-strong) solid var(--accent)"
          : "var(--bw-strong) solid transparent",
        textAlign: "left",
        transition:
          "background var(--dur-fast) var(--ease-linear), color var(--dur-fast) var(--ease-linear)",
        cursor: "pointer",
        position: "relative",
      }}
    >
      {glyph && (
        <Glyph g={glyph} color={active ? "var(--accent)" : "currentColor"} />
      )}
      {!collapsed && (
        <span
          style={{
            flex: 1,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {label}
        </span>
      )}
      {!collapsed && hasBadge && (
        <span
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            fontVariantNumeric: "tabular-nums",
          }}
        >
          {badge}
        </span>
      )}
      {collapsed && hasBadge && (
        // Presence-only badge — a 6px accent dot in the top-right
        // corner. Real counts (numbers, "Off" chips) are unreadable
        // at this width; a dot still tells the user "something here".
        <span
          aria-hidden
          style={{
            position: "absolute",
            top: "var(--sp-6)",
            right: "var(--sp-6)",
            width: "var(--sp-6)",
            height: "var(--sp-6)",
            borderRadius: "var(--r-pill)",
            background: "var(--accent)",
          }}
        />
      )}
      {!collapsed && trailing}
    </button>
  );
}
