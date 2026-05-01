import type { CSSProperties } from "react";
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

const ALL_KIND_OPTIONS: { value: NotificationKind; label: string }[] = [
  { value: "info", label: "Info" },
  { value: "notice", label: "Notice" },
  { value: "error", label: "Error" },
];

const SOURCE_OPTIONS: {
  value: NotificationSource | "both";
  label: string;
}[] = [
  { value: "both", label: "Both" },
  { value: "toast", label: "In-app" },
  { value: "os", label: "OS" },
];

export const WINDOW_OPTIONS: { value: string; label: string }[] = [
  { value: "all", label: "All time" },
  { value: String(60 * 60 * 1000), label: "Last 1 h" },
  { value: String(24 * 60 * 60 * 1000), label: "Last 24 h" },
  { value: String(7 * 24 * 60 * 60 * 1000), label: "Last 7 d" },
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
        placeholder="Search title or body…"
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
        {ALL_KIND_OPTIONS.map((opt) => {
          const active = kinds.has(opt.value);
          return (
            <button
              key={opt.value}
              type="button"
              onClick={() => onToggleKind(opt.value)}
              className="pm-focus"
              style={{
                ...chipStyle,
                background: active ? "var(--accent-soft)" : "transparent",
                color: active ? "var(--accent-ink)" : "var(--fg-muted)",
                borderColor: active ? "var(--accent-border)" : "var(--border)",
              }}
            >
              {opt.label}
            </button>
          );
        })}
        <span style={{ flex: 1 }} />
        <select
          value={source}
          onChange={(e) =>
            onChangeSource(e.target.value as NotificationSource | "both")
          }
          aria-label="Source"
          className="pm-focus"
          style={selectStyle}
        >
          {SOURCE_OPTIONS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
        <select
          value={windowKey}
          onChange={(e) => onChangeWindow(e.target.value)}
          aria-label="Time window"
          className="pm-focus"
          style={selectStyle}
        >
          {WINDOW_OPTIONS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
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
