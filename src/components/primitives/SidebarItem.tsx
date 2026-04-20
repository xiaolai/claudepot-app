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
}: SidebarItemProps) {
  const [hover, setHover] = useState(false);
  const indentPad = indent > 0 ? `calc(var(--sp-10) + var(--sp-14) * ${indent})` : "var(--sp-10)";
  return (
    <button
      type="button"
      title={title}
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      aria-current={active ? "page" : undefined}
      style={{
        width: "100%",
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-10)",
        height: "var(--row-height)",
        padding: `0 var(--sp-10) 0 ${indentPad}`,
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
      }}
    >
      {glyph && (
        <Glyph g={glyph} color={active ? "var(--accent)" : "currentColor"} />
      )}
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
      {badge != null && (
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
      {trailing}
    </button>
  );
}
