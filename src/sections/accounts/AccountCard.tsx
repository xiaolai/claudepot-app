import { type MouseEvent, useState } from "react";
import { Avatar, avatarColorFor } from "../../components/primitives/Avatar";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";
import type { AccountSummary, UsageEntry } from "../../types";
import { AnomalyBanner, isAnomaly } from "./AnomalyBanner";
import { HealthFooter } from "./HealthFooter";
import { UsageBlock } from "./UsageBlock";

interface AccountCardProps {
  account: AccountSummary;
  usageEntry: UsageEntry | null;
  /** True while a long op is running against this account (login). */
  loginBusy?: boolean;
  onRemove: (a: AccountSummary) => void;
  onLogin: (a: AccountSummary) => void;
  onContextMenu?: (e: MouseEvent, a: AccountSummary) => void;
}

/**
 * Per-account usage + health report. Border treatment:
 *   - normal: hairline --line
 *   - bound to a swap target: `--bw-accent` accent left border
 *   - has anomaly: `--bw-accent` warn left border + --warn full border
 *
 * The Remove button is hover-revealed and pinned top-right. The
 * anomaly banner includes its own Re-login CTA that fires the same
 * `onLogin` handler exposed at the card level.
 */
export function AccountCard({
  account: a,
  usageEntry,
  loginBusy,
  onRemove,
  onLogin,
  onContextMenu,
}: AccountCardProps) {
  const [hovered, setHovered] = useState(false);

  const bound = a.is_cli_active || a.is_desktop_active;
  const severe = isAnomaly(a);

  const borderColor = severe
    ? "var(--warn)"
    : bound
      ? "var(--accent-border)"
      : "var(--line)";
  const leftBorder = severe
    ? "var(--bw-accent) solid var(--warn)"
    : bound
      ? "var(--bw-accent) solid var(--accent)"
      : "var(--bw-hair) solid var(--line)";

  const color = avatarColorFor(a.email);

  return (
    <article
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onContextMenu={onContextMenu ? (e) => onContextMenu(e, a) : undefined}
      style={{
        position: "relative",
        background: "var(--bg-raised)",
        border: `var(--bw-hair) solid ${borderColor}`,
        borderLeft: leftBorder,
        borderRadius: "var(--r-3)",
        overflow: "hidden",
        display: "flex",
        flexDirection: "column",
      }}
    >
      {/* remove ✕ — hover-revealed */}
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onRemove(a);
        }}
        title={`Remove ${a.email} — logs out and deletes credentials`}
        aria-label={`Remove ${a.email}`}
        style={{
          position: "absolute",
          top: "var(--sp-10)",
          right: "var(--sp-10)",
          width: "var(--icon-btn-sm)",
          height: "var(--icon-btn-sm)",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          background: hovered ? "var(--bg-active)" : "transparent",
          color: hovered ? "var(--fg-muted)" : "var(--fg-ghost)",
          border: "none",
          borderRadius: "var(--r-1)",
          fontFamily: "inherit",
          fontSize: "var(--fs-xs)",
          lineHeight: "var(--lh-flat)",
          cursor: "pointer",
          opacity: hovered ? 1 : 0,
          transition:
            "opacity var(--dur-base) var(--ease-linear), color var(--dur-hover) var(--ease-linear), background var(--dur-hover) var(--ease-linear)",
          zIndex: "var(--z-popover)" as unknown as number,
        }}
      >
        ✕
      </button>

      {/* identity header */}
      <div
        style={{
          display: "flex",
          alignItems: "flex-start",
          gap: "var(--sp-12)",
          padding: "var(--sp-16) var(--sp-40) var(--sp-14) var(--sp-18)",
          borderBottom: "var(--bw-hair) solid var(--line)",
        }}
      >
        <Avatar name={a.email} color={color} size="xl" />
        <div style={{ flex: 1, overflow: "hidden", minWidth: 0 }}>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-8)",
              minWidth: 0,
            }}
          >
            <span
              style={{
                fontSize: "var(--fs-md)",
                fontWeight: 600,
                letterSpacing: "var(--ls-tight)",
                whiteSpace: "nowrap",
                overflow: "hidden",
                textOverflow: "ellipsis",
              }}
            >
              {a.email}
            </span>
            {a.subscription_type && (
              <span
                className="mono-cap"
                style={{
                  fontSize: "var(--fs-2xs)",
                  color: "var(--fg-ghost)",
                }}
              >
                {a.subscription_type}
              </span>
            )}
          </div>
          {a.org_name && (
            <div
              style={{
                fontSize: "var(--fs-xs)",
                color: "var(--fg-faint)",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {a.org_name}
            </div>
          )}
        </div>
        <div
          style={{
            display: "flex",
            gap: "var(--sp-4)",
            flexShrink: 0,
          }}
        >
          {a.is_cli_active && (
            <Tag tone="accent" glyph={NF.terminal}>
              CLI
            </Tag>
          )}
          {a.is_desktop_active && (
            <Tag tone="accent" glyph={NF.users}>
              Desktop
            </Tag>
          )}
        </div>
      </div>

      {severe && (
        <AnomalyBanner
          account={a}
          onRelogin={() => onLogin(a)}
          disabled={loginBusy}
        />
      )}

      <UsageBlock entry={usageEntry} />
      <HealthFooter account={a} />
    </article>
  );
}
