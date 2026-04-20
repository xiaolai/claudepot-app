import { type CSSProperties, type ReactNode } from "react";
import { Glyph } from "../components/primitives/Glyph";
import { NF } from "../icons";

interface ScreenHeaderProps {
  title: ReactNode;
  subtitle?: ReactNode;
  crumbs?: string[];
  /** Right-aligned actions (buttons, icon buttons). */
  actions?: ReactNode;
  style?: CSSProperties;
}

/**
 * Page-level header — breadcrumbs on top, large title + subtitle on
 * the left, action buttons on the right. Used at the top of every
 * screen's content pane.
 */
export function ScreenHeader({
  title,
  subtitle,
  crumbs,
  actions,
  style,
}: ScreenHeaderProps) {
  return (
    <header
      style={{
        padding: "var(--sp-28) var(--sp-32) var(--sp-20)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        flexShrink: 0,
        ...style,
      }}
    >
      {crumbs && crumbs.length > 0 && (
        <nav
          aria-label="Breadcrumb"
          style={{
            fontSize: "var(--fs-xs)",
            color: "var(--fg-faint)",
            letterSpacing: "var(--ls-wide)",
            textTransform: "uppercase",
            marginBottom: "var(--sp-10)",
            display: "flex",
            alignItems: "center",
            gap: "var(--sp-6)",
          }}
        >
          {crumbs.map((c, i) => (
            <span
              key={`${i}-${c}`}
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: "var(--sp-6)",
              }}
            >
              {i > 0 && (
                <Glyph
                  g={NF.chevronR}
                  color="var(--fg-ghost)"
                  style={{ fontSize: "var(--fs-3xs)" }}
                />
              )}
              <span
                style={{
                  color:
                    i === crumbs.length - 1
                      ? "var(--fg-muted)"
                      : "var(--fg-faint)",
                }}
              >
                {c}
              </span>
            </span>
          ))}
        </nav>
      )}
      <div
        style={{
          display: "flex",
          alignItems: "flex-end",
          gap: "var(--sp-16)",
        }}
      >
        <div style={{ flex: 1, minWidth: 0 }}>
          <h1
            style={{
              fontSize: "var(--fs-xl)",
              fontWeight: 600,
              letterSpacing: "var(--ls-wider)",
              textTransform: "uppercase",
              color: "var(--accent-ink)",
              margin: 0,
            }}
          >
            {title}
          </h1>
          {subtitle && (
            <div
              style={{
                marginTop: "var(--sp-6)",
                fontSize: "var(--fs-sm)",
                color: "var(--fg-muted)",
              }}
            >
              {subtitle}
            </div>
          )}
        </div>
        {actions && (
          <div
            style={{
              display: "flex",
              gap: "var(--sp-8)",
              flexShrink: 0,
            }}
          >
            {actions}
          </div>
        )}
      </div>
    </header>
  );
}
