import { type MouseEvent } from "react";
import { Avatar, avatarColorFor } from "../../components/primitives/Avatar";
import { TargetButton } from "../../components/primitives/TargetButton";
import type { AccountSummary, AppStatus, UsageEntry } from "../../types";
import { AnomalyBanner, isAnomaly } from "./AnomalyBanner";
import { HealthFooter } from "./HealthFooter";
import { UsageBlock } from "./UsageBlock";
import {
  cliTargetProps,
  desktopTargetProps,
  type CliTargetHandlers,
  type DesktopTargetHandlers,
} from "./targetButtonStates";

interface AccountCardProps {
  account: AccountSummary;
  usageEntry: UsageEntry | null;
  status: AppStatus;
  /** True while a long op is running against this account (login). */
  loginBusy?: boolean;
  onLogin: (a: AccountSummary) => void;
  onContextMenu?: (e: MouseEvent, a: AccountSummary) => void;
  cliHandlers: CliTargetHandlers;
  desktopHandlers: DesktopTargetHandlers;
}

/**
 * Per-account usage + health report. Border treatment:
 *   - normal: hairline --line
 *   - bound to a swap target: `--bw-accent` accent left border
 *   - has anomaly: `--bw-accent` warn left border + --warn full border
 *
 * The top-right rail carries two `TargetButton`s — CLI + Desktop —
 * which encode both the slot's activeness *and* the binding verbs.
 * The AnomalyBanner still shows up for drift / rejected / bad creds
 * and fires the same `onLogin` handler.
 */
export function AccountCard({
  account: a,
  usageEntry,
  status,
  loginBusy,
  onLogin,
  onContextMenu,
  cliHandlers,
  desktopHandlers,
}: AccountCardProps) {
  const bound = a.is_cli_active || a.is_desktop_active;
  const severe = isAnomaly(a);

  // One signal per state — never both. Severe dominates (full warn
  // ring). Bound shows as a left accent bar with the rest at
  // hairline. Healthy-unbound is flat line on all edges.
  const borderColor = severe ? "var(--warn)" : "var(--line)";
  const leftBorder = severe
    ? "var(--bw-accent) solid var(--warn)"
    : bound
      ? "var(--bw-accent) solid var(--accent)"
      : "var(--bw-hair) solid var(--line)";

  const color = avatarColorFor(a.email);
  const cliProps = cliTargetProps(a, cliHandlers);
  const desktopProps = desktopTargetProps(a, status, desktopHandlers);

  return (
    <article
      data-account-uuid={a.uuid}
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
      {/* identity header */}
      <div
        style={{
          display: "flex",
          alignItems: "flex-start",
          gap: "var(--sp-12)",
          padding: "var(--sp-16) var(--sp-18) var(--sp-14) var(--sp-18)",
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
            gap: "var(--sp-6)",
            flexShrink: 0,
            alignItems: "center",
          }}
        >
          <TargetButton {...cliProps} />
          {desktopProps && <TargetButton {...desktopProps} />}
        </div>
      </div>

      {severe && (
        <AnomalyBanner
          account={a}
          onRelogin={() => onLogin(a)}
          disabled={loginBusy}
        />
      )}

      <UsageBlock entry={usageEntry} anomalyShown={severe} />
      <HealthFooter account={a} />
    </article>
  );
}
