import type { CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import type {
  NotificationKind,
  NotificationSource,
} from "../api/notification";

/**
 * Filter row for the notification log popover. Lifted into its own
 * file so the parent stays under the loc-guardian limit and so the
 * filter shape can be exercised in isolation.
 *
 * Three independent axes:
 *
 *   - kind chips (info / notice / error) — multi-select, AND'd
 *     against the row.
 *   - source select (both / in-app / OS) — single-select, gated.
 *   - window select (all / 1h / 24h / 7d) — single-select, gated.
 *
 * Plus a free-text search over title + body. All four combine into
 * a `NotificationLogFilter` in the parent's `useMemo`.
 */

const KIND_OPTIONS: NotificationKind[] = ["info", "notice", "error"];

const SOURCE_OPTIONS: (NotificationSource | "both")[] = [
  "both",
  "toast",
  "os",
];

export const WINDOW_OPTIONS: { value: string; labelKey: string }[] = [
  { value: "all", labelKey: "shell.notification.windowAll" },
  { value: String(60 * 60 * 1000), labelKey: "shell.notification.window1h" },
  { value: String(24 * 60 * 60 * 1000), labelKey: "shell.notification.window24h" },
  { value: String(7 * 24 * 60 * 60 * 1000), labelKey: "shell.notification.window7d" },
];

interface PopoverFiltersProps {
  kinds: Set<NotificationKind>;
  onToggleKind: (k: NotificationKind) => void;
  source: NotificationSource | "both";
  onChangeSource: (s: NotificationSource | "both") => void;
  windowKey: string;
  onChangeWindow: (k: string) => void;
  query: string;
  onChangeQuery: (q: string) => void;
}

const kindLabelKey: Record<NotificationKind, string> = {
  info: "shell.notification.filterInfo",
  notice: "shell.notification.filterNotice",
  error: "shell.notification.filterError",
};

const sourceLabelKey: Record<NotificationSource | "both", string> = {
  both: "shell.notification.sourceBoth",
  toast: "shell.notification.sourceInApp",
  os: "shell.notification.sourceOs",
};

export function NotificationLogFilters({
  kinds,
  onToggleKind,
  source,
  onChangeSource,
  windowKey,
  onChangeWindow,
  query,
  onChangeQuery,
}: PopoverFiltersProps) {
  const { t } = useTranslation();
  return (
    <div
      style={{
        padding: "var(--sp-8) var(--sp-12) var(--sp-10)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: "var(--bg)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-6)",
      }}
    >
      <input
        type="search"
        value={query}
        placeholder={t("shell.notification.searchPlaceholder")}
        onChange={(e) => onChangeQuery(e.target.value)}
        className="pm-focus"
        style={{
          width: "100%",
          fontSize: "var(--fs-xs)",
          padding: "var(--sp-4) var(--sp-8)",
          border: "var(--bw-subhair) solid var(--border)",
          borderRadius: "var(--r-1)",
          background: "var(--bg-sunken)",
          color: "var(--fg)",
          fontFamily: "inherit",
        }}
      />
      <div
        style={{
          display: "flex",
          gap: "var(--sp-4)",
          flexWrap: "wrap",
          alignItems: "center",
        }}
      >
        {KIND_OPTIONS.map((kind) => {
          const active = kinds.has(kind);
          const label = t(kindLabelKey[kind]);
          return (
            <button
              key={kind}
              type="button"
              onClick={() => onToggleKind(kind)}
              className="pm-focus"
              style={{
                ...chipStyle,
                background: active ? "var(--accent-soft)" : "transparent",
                color: active ? "var(--accent-ink)" : "var(--fg-muted)",
                borderColor: active ? "var(--accent-border)" : "var(--border)",
              }}
            >
              {label}
            </button>
          );
        })}
        <span style={{ flex: 1 }} />
        <select
          value={source}
          onChange={(e) =>
            onChangeSource(e.target.value as NotificationSource | "both")
          }
          aria-label={t("shell.notification.sourceLabel")}
          className="pm-focus"
          style={selectStyle}
        >
          {SOURCE_OPTIONS.map((s) => (
            <option key={s} value={s}>
              {t(sourceLabelKey[s])}
            </option>
          ))}
        </select>
        <select
          value={windowKey}
          onChange={(e) => onChangeWindow(e.target.value)}
          aria-label={t("shell.notification.windowLabel")}
          className="pm-focus"
          style={selectStyle}
        >
          {WINDOW_OPTIONS.map((o) => (
            <option key={o.value} value={o.value}>
              {t(o.labelKey)}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}

const chipStyle: CSSProperties = {
  fontSize: "var(--fs-3xs)",
  padding: "var(--sp-2) var(--sp-6)",
  border: "var(--bw-subhair) solid var(--border)",
  borderRadius: "var(--r-pill)",
  background: "transparent",
  color: "var(--fg-muted)",
  cursor: "pointer",
  fontFamily: "inherit",
  lineHeight: 1.3,
};

const selectStyle: CSSProperties = {
  fontSize: "var(--fs-3xs)",
  padding: "var(--sp-2) var(--sp-4)",
  border: "var(--bw-subhair) solid var(--border)",
  borderRadius: "var(--r-1)",
  background: "var(--bg-sunken)",
  color: "var(--fg-muted)",
  cursor: "pointer",
  fontFamily: "inherit",
};
