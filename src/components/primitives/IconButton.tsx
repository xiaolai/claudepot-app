import { type CSSProperties, type MouseEvent, useState } from "react";
import type { NfIcon } from "../../icons";
import { Glyph } from "./Glyph";

interface IconButtonProps {
  glyph: NfIcon;
  onClick?: () => void;
  /**
   * Passthrough for `mousedown` — mainly for Tauri drag regions,
   * where an interactive child must stop propagation so the
   * surrounding `data-tauri-drag-region` doesn't swallow the click.
   */
  onMouseDown?: (e: MouseEvent<HTMLButtonElement>) => void;
  active?: boolean;
  disabled?: boolean;
  /**
   * Square size. Semantic variants map to the `--icon-btn-*` tokens
   * (sm = 22, md = 28, lg = 32). Numeric values are an escape hatch
   * — they bypass the token set and should be avoided.
   */
  size?: "sm" | "md" | "lg" | number;
  title?: string;
  "aria-label"?: string;
  "aria-pressed"?: boolean;
  "aria-haspopup"?: boolean | "menu" | "listbox" | "dialog";
  "aria-expanded"?: boolean;
  style?: CSSProperties;
}

const SIZE_TOKEN: Record<"sm" | "md" | "lg", string> = {
  sm: "var(--icon-btn-sm)",
  md: "var(--icon-btn-md)",
  lg: "var(--icon-btn-lg)",
};

function resolveSize(size: IconButtonProps["size"]): string {
  if (typeof size === "string") return SIZE_TOKEN[size];
  if (typeof size === "number") return `${size}px`;
  return SIZE_TOKEN.md;
}

/**
 * Square, icon-only button. `size` defaults to `"md"` (28px, matching
 * the body row height). `"sm"` for compact toolbars, `"lg"` for large
 * controls. Numeric sizes are legacy and should be migrated to the
 * semantic variants.
 *
 * For labeled buttons, reach for `<Button glyph={…}>Label</Button>`.
 */
export function IconButton({
  glyph,
  onClick,
  onMouseDown,
  active,
  disabled,
  size = "md",
  title,
  style,
  ...aria
}: IconButtonProps) {
  const [hover, setHover] = useState(false);
  const dim = resolveSize(size);
  return (
    <button
      type="button"
      title={title}
      disabled={disabled}
      onClick={disabled ? undefined : onClick}
      onMouseDown={onMouseDown}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      className="pm-focus"
      {...aria}
      style={{
        width: dim,
        height: dim,
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        borderRadius: "var(--r-2)",
        background: active
          ? "var(--bg-active)"
          : hover
            ? "var(--bg-hover)"
            : "transparent",
        color: active ? "var(--accent-ink)" : "var(--fg-muted)",
        opacity: disabled ? "var(--opacity-disabled)" : 1,
        cursor: disabled ? "not-allowed" : "pointer",
        transition:
          "background var(--dur-fast) var(--ease-linear), color var(--dur-fast) var(--ease-linear)",
        border: "none",
        flexShrink: 0,
        ...style,
      }}
    >
      <Glyph g={glyph} />
    </button>
  );
}
