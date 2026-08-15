import type { CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { Glyph } from "../components/primitives/Glyph";
import { NF } from "../icons";
import type { NotificationLogOrder } from "../api/notification";

/**
 * Title strip + action cluster for the notification log popover.
 * Separated from the popover so the popover stays under loc-guardian
 * limits and so the action cluster can grow without bloating its
 * parent.
 *
 * Buttons:
 *   - Sort toggle (newest ⇄ oldest) — chevron rotates with order.
 *   - Mark read — clears the unread badge for entries the user
 *     scrolled past without re-opening.
 *   - Clear — wipes the log; gated by the popover's ConfirmDialog.
 */

interface PopoverHeaderProps {
  order: NotificationLogOrder;
  onToggleOrder: () => void;
  onMarkAllRead: () => void;
  onClear: () => void;
  hasEntries: boolean;
}

export function NotificationLogHeader({
  order,
  onToggleOrder,
  onMarkAllRead,
  onClear,
  hasEntries,
}: PopoverHeaderProps) {
  const { t } = useTranslation("components");
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-6)",
        padding: "var(--sp-10) var(--sp-12)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: "var(--bg)",
      }}
    >
      <h2
        className="mono-cap"
        style={{
          flex: 1,
          margin: 0,
          fontSize: "var(--fs-2xs)",
          fontWeight: 500,
          color: "var(--fg-muted)",
        }}
      >
        {t("notifLog.panel")}
      </h2>
      <button
        type="button"
        onClick={onToggleOrder}
        className="pm-focus"
        title={
          order === "newestFirst"
            ? t("notifLog.sortOldest")
            : t("notifLog.sortNewest")
        }
        style={miniBtnStyle}
      >
        <Glyph
          g={order === "newestFirst" ? NF.chevronD : NF.chevronU}
          size="var(--fs-sm)"
        />
        <span>
          {order === "newestFirst"
            ? t("notifLog.newest")
            : t("notifLog.oldest")}
        </span>
      </button>
      <button
        type="button"
        onClick={onMarkAllRead}
        disabled={!hasEntries}
        className="pm-focus"
        title={t("notifLog.markReadTitle")}
        style={miniBtnStyle}
      >
        {t("notifLog.markRead")}
      </button>
      <button
        type="button"
        onClick={onClear}
        disabled={!hasEntries}
        className="pm-focus"
        title={t("notifLog.clearTitle")}
        style={miniBtnStyle}
      >
        {t("notifLog.clear")}
      </button>
    </div>
  );
}

const miniBtnStyle: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: "var(--sp-4)",
  fontSize: "var(--fs-2xs)",
  padding: "var(--sp-2) var(--sp-6)",
  background: "transparent",
  border: "var(--bw-subhair) solid var(--line)",
  borderRadius: "var(--r-1)",
  color: "var(--fg-muted)",
  cursor: "pointer",
  fontFamily: "inherit",
  lineHeight: 1.3,
};
