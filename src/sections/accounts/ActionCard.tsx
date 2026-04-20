import type { ReactNode } from "react";
import type { NfIcon } from "../../icons";
import { Button } from "../../components/primitives/Button";
import { DevBadge } from "../../components/primitives/DevBadge";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";

interface ActionCardProps {
  glyph: NfIcon;
  title: string;
  subtitle: ReactNode;
  /** Backend command name, shown as a DevBadge when Developer mode is on. */
  command?: string;
  disabled?: boolean;
  /** Hint explaining why the card is disabled; shown inline when so. */
  disabledHint?: string;
  onClick?: () => void;
  cta: string;
  ctaGlyph?: NfIcon;
  /** Accent styling — used for the primary action in a modal. */
  accent?: boolean;
  children?: ReactNode;
}

/**
 * Paired title + subtitle + CTA button, with an optional child region
 * (e.g. an identity preview). A disabled card shows a dashed-border
 * badge and an info strip explaining why; the `command` prop surfaces
 * the backend command name as a DevBadge when Developer mode is on.
 */
export function ActionCard({
  glyph,
  title,
  subtitle,
  command,
  disabled,
  disabledHint,
  onClick,
  cta,
  ctaGlyph,
  accent,
  children,
}: ActionCardProps) {
  return (
    <div
      style={{
        border: `var(--bw-hair) solid ${accent && !disabled ? "var(--accent-border)" : "var(--line)"}`,
        borderRadius: "var(--r-2)",
        padding: "var(--sp-14)",
        background:
          accent && !disabled ? "var(--accent-soft)" : "var(--bg-sunken)",
        opacity: disabled ? "var(--opacity-dimmed)" : 1,
        transition:
          "opacity var(--dur-hover) var(--ease-linear), background var(--dur-hover) var(--ease-linear)",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "flex-start",
          gap: "var(--sp-12)",
        }}
      >
        <div
          aria-hidden
          style={{
            width: "var(--sp-28)",
            height: "var(--sp-28)",
            flexShrink: 0,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            background: "var(--bg)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-1)",
            color: "var(--fg-muted)",
          }}
        >
          <Glyph g={glyph} />
        </div>

        <div style={{ flex: 1, minWidth: 0 }}>
          <div
            style={{
              display: "flex",
              alignItems: "baseline",
              gap: "var(--sp-8)",
              flexWrap: "wrap",
            }}
          >
            <span
              style={{
                fontSize: "var(--fs-sm)",
                fontWeight: 600,
                letterSpacing: "var(--ls-tight)",
                color: "var(--fg)",
              }}
            >
              {title}
            </span>
            {command && <DevBadge>{command}</DevBadge>}
          </div>
          <div
            style={{
              fontSize: "var(--fs-xs)",
              color: "var(--fg-muted)",
              marginTop: "var(--sp-3)",
              lineHeight: "var(--lh-body)",
            }}
          >
            {subtitle}
          </div>
        </div>

        <div style={{ flexShrink: 0 }}>
          {disabled ? (
            <span
              title={disabledHint}
              className="mono-cap"
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: "var(--sp-4)",
                height: "var(--btn-h-md)",
                padding: "0 var(--sp-10)",
                fontSize: "var(--fs-xs)",
                color: "var(--fg-ghost)",
                border: "var(--bw-hair) dashed var(--line-strong)",
                borderRadius: "var(--r-1)",
                cursor: disabledHint ? "help" : "not-allowed",
              }}
            >
              {ctaGlyph && <Glyph g={ctaGlyph} />}
              {cta}
            </span>
          ) : (
            <Button
              variant={accent ? "solid" : "ghost"}
              glyph={ctaGlyph}
              onClick={onClick}
            >
              {cta}
            </Button>
          )}
        </div>
      </div>

      {children}

      {disabled && disabledHint && (
        <div
          style={{
            marginTop: "var(--sp-10)",
            padding: "var(--sp-8) var(--sp-10)",
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            lineHeight: "var(--lh-body)",
            background: "var(--bg)",
            border: "var(--bw-hair) dashed var(--line)",
            borderRadius: "var(--r-1)",
            display: "flex",
            gap: "var(--sp-8)",
            alignItems: "flex-start",
          }}
        >
          <Glyph
            g={NF.info}
            style={{
              marginTop: "var(--sp-px)",
              color: "var(--fg-ghost)",
            }}
          />
          <span>{disabledHint}</span>
        </div>
      )}
    </div>
  );
}
