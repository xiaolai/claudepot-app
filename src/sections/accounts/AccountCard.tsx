import { type MouseEvent } from "react";
import { Avatar, avatarColorFor } from "../../components/primitives/Avatar";
import { IconButton } from "../../components/primitives/IconButton";
import { TargetButton } from "../../components/primitives/TargetButton";
import { NF } from "../../icons";
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

  // Keyboard-focusable context-menu affordance. The inline Remove
  // button went away with Tier 3-D, and the in-card CLI/Desktop
  // TargetButtons cover the common verbs. Destructive actions
  // (remove account, forget desktop snapshot) live only in the
  // context menu now, so the card itself must be reachable by
  // keyboard and open that menu from Shift+F10 / the Menu key /
  // Enter. `article` with `tabIndex=0` makes it focusable; the
  // key handler synthesises a context-menu event anchored on the
  // card's own top-left so the popover still appears in a sane
  // place when triggered without a mouse.
  const handleKeyboardMenu = (e: React.KeyboardEvent<HTMLElement>) => {
    if (!onContextMenu) return;
    const isMenuKey = e.key === "ContextMenu";
    const isShiftF10 = e.key === "F10" && e.shiftKey;
    if (!isMenuKey && !isShiftF10) return;
    e.preventDefault();
    const rect = e.currentTarget.getBoundingClientRect();
    onContextMenu(
      {
        preventDefault: () => {},
        clientX: rect.left + 12,
        clientY: rect.top + 12,
      } as unknown as React.MouseEvent,
      a,
    );
  };

  return (
    <article
      data-account-uuid={a.uuid}
      tabIndex={0}
      aria-label={`Account ${a.email}`}
      onContextMenu={onContextMenu ? (e) => onContextMenu(e, a) : undefined}
      onKeyDown={handleKeyboardMenu}
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
          {onContextMenu && (
            <IconButton
              glyph={NF.ellipsis}
              size="sm"
              onClick={() => {
                // Anchor the menu at the button's position — the
                // click event's synthetic coords are enough to land
                // the popover near where the user expects.
                const el = document.activeElement as HTMLElement | null;
                const rect = el?.getBoundingClientRect();
                onContextMenu(
                  {
                    preventDefault: () => {},
                    clientX: rect ? rect.right : 0,
                    clientY: rect ? rect.bottom : 0,
                  } as unknown as MouseEvent,
                  a,
                );
              }}
              title="More actions"
              aria-label={`More actions for ${a.email}`}
              aria-haspopup="menu"
            />
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

      <UsageBlock entry={usageEntry} anomalyShown={severe} />
      <HealthFooter account={a} />
    </article>
  );
}
