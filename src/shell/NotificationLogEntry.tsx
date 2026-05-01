import type {
  NotificationEntry,
  NotificationKind,
} from "../api/notification";

/**
 * Single-row presentation for a notification log entry. Lifted into
 * its own file so the popover stays under the loc-guardian limit.
 *
 * Visual conventions:
 *   - Severity color lives on a 6 px dot at the row start.
 *     Color-only signal is paired with the textual `kind` token in
 *     the meta line, satisfying design.md's "color never alone."
 *   - Title gets one line; body, if present, gets one line; meta
 *     row carries source + kind + click-affordance.
 *   - Rows with a non-null target are clickable; cursor + hover
 *     background only on those.
 */

interface EntryRowProps {
  entry: NotificationEntry;
  onClick: () => void;
}

export function NotificationLogEntry({ entry, onClick }: EntryRowProps) {
  const clickable = entry.target != null;
  const colors = kindColors(entry.kind);
  // Keyboard activation for clickable rows. Pre-fix this was a bare
  // `<li onClick>` — mouse-only, failing the project's a11y floor
  // ("every interactive element is keyboard-reachable"). Promoting
  // the row to role="button" + tabIndex + Enter/Space keeps the
  // <li> semantics for screen readers (this is a list of
  // notifications) while still letting the user activate without a
  // mouse. Non-clickable rows stay role-less; they have nothing to
  // activate.
  const interactiveProps = clickable
    ? {
        role: "button" as const,
        tabIndex: 0,
        onClick,
        onKeyDown: (e: React.KeyboardEvent<HTMLLIElement>) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onClick();
          }
        },
      }
    : {};
  return (
    <li
      {...interactiveProps}
      className={clickable ? "pm-focus" : undefined}
      style={{
        padding: "var(--sp-8) var(--sp-12)",
        borderBottom: "var(--bw-subhair) solid var(--border)",
        cursor: clickable ? "pointer" : "default",
        display: "flex",
        gap: "var(--sp-10)",
        alignItems: "flex-start",
      }}
      onMouseEnter={
        clickable
          ? (e) => {
              e.currentTarget.style.background = "var(--bg-hover)";
            }
          : undefined
      }
      onMouseLeave={
        clickable
          ? (e) => {
              e.currentTarget.style.background = "transparent";
            }
          : undefined
      }
    >
      <span
        aria-hidden
        style={{
          width: "var(--sp-6)",
          height: "var(--sp-6)",
          marginTop: "var(--sp-6)",
          borderRadius: "var(--r-pill)",
          background: colors.dot,
          flexShrink: 0,
        }}
      />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            gap: "var(--sp-8)",
            alignItems: "baseline",
          }}
        >
          <span
            style={{
              fontSize: "var(--fs-xs)",
              fontWeight: 500,
              color: "var(--fg)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {entry.title}
          </span>
          <time
            style={{
              fontSize: "var(--fs-3xs)",
              color: "var(--fg-faint)",
              flexShrink: 0,
              fontVariantNumeric: "tabular-nums",
            }}
          >
            {formatTime(entry.ts_ms)}
          </time>
        </div>
        {entry.body && (
          <div
            style={{
              fontSize: "var(--fs-2xs)",
              color: "var(--fg-muted)",
              marginTop: "var(--sp-2)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {entry.body}
          </div>
        )}
        <div
          style={{
            display: "flex",
            gap: "var(--sp-6)",
            marginTop: "var(--sp-2)",
            fontSize: "var(--fs-3xs)",
            color: "var(--fg-faint)",
          }}
        >
          <span>{entry.source === "toast" ? "in-app" : "OS"}</span>
          <span>·</span>
          <span>{entry.kind}</span>
          {clickable && (
            <>
              <span>·</span>
              <span>click to follow</span>
            </>
          )}
        </div>
      </div>
    </li>
  );
}

function kindColors(k: NotificationKind): { dot: string } {
  switch (k) {
    case "error":
      return { dot: "var(--danger)" };
    case "notice":
      return { dot: "var(--accent)" };
    case "info":
    default:
      return { dot: "var(--fg-faint)" };
  }
}

/**
 * Localized relative-time formatter. Goes from "just now" (under one
 * minute) to "Nm" minutes (sub-hour) to a wall-clock time today, to
 * a Mar 4 style date past today. Keeps the UI compact in the row's
 * tight time slot.
 */
function formatTime(ms: number): string {
  const d = new Date(ms);
  const now = new Date();
  const diff = (now.getTime() - ms) / 1000;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  const sameDay = d.toDateString() === now.toDateString();
  if (sameDay) {
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
  return d.toLocaleDateString([], {
    month: "short",
    day: "numeric",
  });
}
