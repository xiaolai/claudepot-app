import { type ReactNode, useEffect } from "react";
import type { NfIcon } from "../../icons";
import { Glyph } from "./Glyph";
import { IconButton } from "./IconButton";
import { NF } from "../../icons";

interface ModalProps {
  open: boolean;
  onClose?: () => void;
  /**
   * Modal width — named variants map to `--modal-width-*` tokens
   * (sm = 440, md = 480, lg = 520). Numeric sizes are allowed but
   * bypass the token set.
   */
  width?: "sm" | "md" | "lg" | number;
  children: ReactNode;
  /** Optional aria-labelledby target for the header element id. */
  "aria-labelledby"?: string;
}

const WIDTH_TOKEN: Record<"sm" | "md" | "lg", string> = {
  sm: "var(--modal-width-sm)",
  md: "var(--modal-width-md)",
  lg: "var(--modal-width-lg)",
};

function resolveWidth(width: ModalProps["width"]): string {
  if (typeof width === "string") return WIDTH_TOKEN[width];
  if (typeof width === "number") return `${width}px`;
  return WIDTH_TOKEN.md;
}

/**
 * Centered dialog with a dimmed, blurred scrim. Closes on scrim click
 * or Escape. Use for blocking flows only (destructive confirmations,
 * add-account). Never for completion messages — use a toast.
 */
export function Modal({
  open,
  onClose,
  width,
  children,
  ...aria
}: ModalProps) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose?.();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      onClick={onClose}
      style={{
        position: "fixed",
        inset: 0,
        zIndex: "var(--z-modal)" as unknown as number,
        background: "var(--scrim)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        padding: "var(--sp-32)",
        backdropFilter: "blur(var(--backdrop-blur-sm))",
        WebkitBackdropFilter: "blur(var(--backdrop-blur-sm))",
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        {...aria}
        style={{
          width: resolveWidth(width),
          maxWidth: "100%",
          maxHeight: "100%",
          background: "var(--bg)",
          border: "var(--bw-hair) solid var(--line-strong)",
          borderRadius: "var(--r-3)",
          boxShadow: "var(--shadow-modal)",
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
          fontFamily: "var(--font)",
        }}
      >
        {children}
      </div>
    </div>
  );
}

interface ModalHeaderProps {
  glyph?: NfIcon;
  title: ReactNode;
  onClose?: () => void;
  /** Anchor for `aria-labelledby`. */
  id?: string;
}

export function ModalHeader({ glyph, title, onClose, id }: ModalHeaderProps) {
  return (
    <div
      style={{
        padding: "var(--sp-14) var(--sp-18) var(--sp-12)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-10)",
      }}
    >
      {glyph && (
        <Glyph
          g={glyph}
          color="var(--fg-muted)"
          size="var(--fs-sm)"
        />
      )}
      <span
        id={id}
        className="mono-cap"
        style={{
          flex: 1,
          color: "var(--fg)",
          fontSize: "var(--fs-xs)",
        }}
      >
        {title}
      </span>
      {onClose && (
        <IconButton
          glyph={NF.x}
          onClick={onClose}
          size={22}
          title="Close (Esc)"
          aria-label="Close"
        />
      )}
    </div>
  );
}

interface ModalBodyProps {
  children: ReactNode;
  style?: React.CSSProperties;
}

export function ModalBody({ children, style }: ModalBodyProps) {
  return (
    <div
      style={{
        padding: "var(--sp-20) var(--sp-22)",
        overflow: "auto",
        ...style,
      }}
    >
      {children}
    </div>
  );
}

export function ModalFooter({ children }: { children: ReactNode }) {
  return (
    <div
      style={{
        padding: "var(--sp-12) var(--sp-18)",
        borderTop: "var(--bw-hair) solid var(--line)",
        background: "var(--bg-raised)",
        display: "flex",
        alignItems: "center",
        justifyContent: "flex-end",
        gap: "var(--sp-8)",
      }}
    >
      {children}
    </div>
  );
}
