import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, MouseEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { IconButton } from "../components/primitives/IconButton";
import { Glyph } from "../components/primitives/Glyph";
import { Button } from "../components/primitives/Button";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { NF } from "../icons";
import { api } from "../api";
import { consumeRecentTarget } from "../lib/notify";
import type {
  NotificationEntry,
  NotificationKind,
  NotificationLogFilter,
  NotificationLogOrder,
  NotificationSource,
} from "../api/notification";

/**
 * Bell icon + popover panel for the in-app notification log.
 *
 * The popover is the user-facing view of the persistent ring buffer
 * at `~/.claudepot/notifications.json`. Every `pushToast` and every
 * `dispatchOsNotification` lands one entry in that buffer, regardless
 * of which surface(s) actually fired (toast when focused, OS banner
 * when blurred — sometimes both for the same logical event). The
 * popover's Source filter exposes that axis.
 *
 * Why bell-in-WindowChrome rather than a top-level section: the log
 * is meta-UI ("what did this app just tell me?"), not a domain noun
 * (account/cli/desktop/project). Adding a section would inflate the
 * primary nav for content that's read-only and bursty. A chrome icon
 * with an unread badge — the standard surface for this in every
 * desktop app since Mac OS X 10.5 — fits the actual cognitive load.
 *
 * Polling cadence: 8 s. The capture sites are fire-and-forget IPC
 * calls so a freshly-dispatched entry can take a tick to surface;
 * 8 s is short enough that the badge feels live but doesn't burn
 * cycles when nothing's happening. Window focus also triggers an
 * immediate refresh — the user came back, show them what they
 * missed.
 */

interface NotificationBellProps {
  /** Stop drag from propagating to the WindowChrome's tauri-drag
   *  region. Same shape as the theme toggle's `onMouseDown` hook. */
  onMouseDown?: (e: MouseEvent<HTMLButtonElement>) => void;
}

export function NotificationBell({ onMouseDown }: NotificationBellProps) {
  const [open, setOpen] = useState(false);
  const [unread, setUnread] = useState(0);
  const buttonRef = useRef<HTMLButtonElement | null>(null);

  const refreshUnread = useCallback(async () => {
    try {
      const n = await api.notificationLogUnreadCount();
      setUnread(n);
    } catch {
      // Bell stays on its last-known count — a transient IPC blip
      // shouldn't blank the badge.
    }
  }, []);

  useEffect(() => {
    void refreshUnread();
    const t = setInterval(() => void refreshUnread(), 8_000);
    const onFocus = () => void refreshUnread();
    window.addEventListener("focus", onFocus);
    return () => {
      clearInterval(t);
      window.removeEventListener("focus", onFocus);
    };
  }, [refreshUnread]);

  // Listen for the dispatcher's custom signal so the badge updates
  // synchronously when a new notification fires from this same
  // process — no need to wait for the 8 s tick.
  useEffect(() => {
    const handler = () => void refreshUnread();
    window.addEventListener("claudepot:notification-logged", handler);
    return () =>
      window.removeEventListener("claudepot:notification-logged", handler);
  }, [refreshUnread]);

  // Cross-window log mutations (Cleanup → Clear, future surfaces).
  // Subscribe lazily so we only carry the listener when one is
  // actually needed.
  useEffect(() => {
    const unsub = listen("notification-log-changed", () => {
      void refreshUnread();
    });
    return () => {
      void unsub.then((u) => u());
    };
  }, [refreshUnread]);

  const onToggle = useCallback(() => {
    setOpen((o) => !o);
  }, []);

  const badge = unread > 99 ? "99+" : String(unread);

  return (
    <>
      <div style={{ position: "relative", display: "inline-flex" }}>
        <IconButton
          ref={buttonRef}
          glyph={NF.bell}
          onClick={onToggle}
          onMouseDown={onMouseDown}
          title={
            unread > 0
              ? `Notifications (${unread} unread)`
              : "Notifications"
          }
          aria-label="Open notifications"
          aria-haspopup="dialog"
          aria-expanded={open}
          style={{ fontSize: "var(--fs-md)" }}
        />
        {unread > 0 && (
          <span
            aria-hidden
            style={{
              position: "absolute",
              top: 2,
              right: 2,
              minWidth: "var(--sp-12)",
              height: "var(--sp-12)",
              padding: "0 tokens.sp[3]",
              borderRadius: "var(--r-pill)",
              background: "var(--accent)",
              color: "var(--accent-text)",
              fontSize: 9,
              fontWeight: 600,
              lineHeight: "var(--sp-12)",
              textAlign: "center",
              fontVariantNumeric: "tabular-nums",
              pointerEvents: "none",
            }}
          >
            {badge}
          </span>
        )}
      </div>
      {open && (
        <NotificationLogPopover
          anchorRef={buttonRef}
          onClose={() => setOpen(false)}
          onCountMaybeChanged={refreshUnread}
        />
      )}
    </>
  );
}

// ── Popover ──────────────────────────────────────────────────────

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

const WINDOW_OPTIONS: { value: string; label: string }[] = [
  { value: "all", label: "All time" },
  { value: String(60 * 60 * 1000), label: "Last 1 h" },
  { value: String(24 * 60 * 60 * 1000), label: "Last 24 h" },
  { value: String(7 * 24 * 60 * 60 * 1000), label: "Last 7 d" },
];

const POPOVER_WIDTH = 380;
const POPOVER_GAP = 8;

interface PopoverProps {
  anchorRef: React.RefObject<HTMLButtonElement | null>;
  onClose: () => void;
  onCountMaybeChanged: () => void;
}

function NotificationLogPopover({
  anchorRef,
  onClose,
  onCountMaybeChanged,
}: PopoverProps) {
  const panelRef = useRef<HTMLDivElement | null>(null);
  const [entries, setEntries] = useState<NotificationEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);

  // Filter / sort state. Lives inside the popover — kept local so
  // closing the popover wipes the filter (matches user expectation:
  // re-opening starts fresh) and so the bell never carries stale
  // filter state.
  const [kinds, setKinds] = useState<Set<NotificationKind>>(new Set());
  const [source, setSource] = useState<NotificationSource | "both">("both");
  const [windowKey, setWindowKey] = useState<string>("all");
  const [query, setQuery] = useState<string>("");
  const [order, setOrder] = useState<NotificationLogOrder>("newestFirst");

  const filter = useMemo<NotificationLogFilter>(() => {
    const f: NotificationLogFilter = {};
    if (kinds.size > 0) f.kinds = Array.from(kinds);
    if (source !== "both") f.source = source;
    if (windowKey !== "all") f.sinceMs = Date.now() - parseInt(windowKey, 10);
    const trimmed = query.trim();
    if (trimmed) f.query = trimmed;
    return f;
  }, [kinds, source, windowKey, query]);

  // Position. Anchor to the right of the button, below the chrome.
  const [pos, setPos] = useState<{ top: number; right: number } | null>(null);
  useEffect(() => {
    const compute = () => {
      const rect = anchorRef.current?.getBoundingClientRect();
      if (!rect) return;
      // Right-anchor by viewport edge so the panel never clips off-
      // screen on a narrow window — we measure distance from the
      // right edge to the button's right edge, then push the panel
      // that far in (so its own right edge aligns with the bell).
      const right = Math.max(window.innerWidth - rect.right, POPOVER_GAP);
      const top = rect.bottom + POPOVER_GAP;
      setPos({ top, right });
    };
    compute();
    window.addEventListener("resize", compute);
    return () => window.removeEventListener("resize", compute);
  }, [anchorRef]);

  const refresh = useCallback(async () => {
    setError(null);
    try {
      const list = await api.notificationLogList(filter, order, 200);
      setEntries(list);
      setLoading(false);
    } catch (e) {
      setError(String(e));
      setLoading(false);
    }
  }, [filter, order]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Mark-all-read when the popover OPENS — that's the user's
  // "I'm looking at this now" moment. We don't wait for them to
  // click a button; the unread badge is supposed to mean "you have
  // not looked," and the popover-open IS looking. The Mark all read
  // button exists only because some users will close the popover
  // without scrolling and want the badge to clear retroactively
  // (which mark-on-open already does), so the button is there as a
  // "force the count to 0 for entries I scrolled past." Same effect
  // either way — kept for affordance discoverability.
  useEffect(() => {
    void api
      .notificationLogMarkAllRead()
      .then(() => onCountMaybeChanged())
      .catch(() => {
        /* swallow */
      });
  }, [onCountMaybeChanged]);

  // Close on outside click. Reuses the same pattern as ContextMenu —
  // mousedown so we close before any down-stream click handlers fire.
  useEffect(() => {
    const onMouseDown = (e: globalThis.MouseEvent) => {
      const t = e.target as Node | null;
      if (!t) return;
      if (panelRef.current?.contains(t)) return;
      if (anchorRef.current?.contains(t)) return;
      onClose();
    };
    document.addEventListener("mousedown", onMouseDown);
    return () => document.removeEventListener("mousedown", onMouseDown);
  }, [anchorRef, onClose]);

  // Esc closes.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const toggleKind = useCallback((k: NotificationKind) => {
    setKinds((prev) => {
      const next = new Set(prev);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });
  }, []);

  const onMarkAllRead = useCallback(async () => {
    try {
      await api.notificationLogMarkAllRead();
      onCountMaybeChanged();
    } catch (e) {
      setError(String(e));
    }
  }, [onCountMaybeChanged]);

  const onClear = useCallback(async () => {
    setConfirmClear(false);
    try {
      await api.notificationLogClear();
      await refresh();
      onCountMaybeChanged();
    } catch (e) {
      setError(String(e));
    }
  }, [refresh, onCountMaybeChanged]);

  const onEntryClick = useCallback(
    (entry: NotificationEntry) => {
      // Re-route through the same NotificationTarget discriminator
      // the live click queue uses. The dispatcher's
      // consumeRecentTarget is for live banner clicks; here we
      // simulate the same flow by stashing the entry's target
      // (if any) into the queue and then dispatching the focus event.
      if (!entry.target) return;
      // Push the entry's stored target onto the queue so the App
      // shell's focus listener picks it up identically to how it
      // would for a fresh banner click. We do this via a window
      // event because the queue lives in `lib/notify.ts` as
      // module-private state — re-using its `consumeRecentTarget`
      // surface keeps the routing logic in one place.
      window.dispatchEvent(
        new CustomEvent("claudepot:notification-log-target", {
          detail: { target: entry.target },
        }),
      );
      onClose();
    },
    [onClose],
  );

  // Used inside `onEntryClick`'s downstream listener (App.tsx) — see
  // the consumer there. We expose `consumeRecentTarget` for parity
  // even though it isn't called from this file.
  void consumeRecentTarget;

  if (!pos) return null;

  return (
    <>
      <div
        ref={panelRef}
        role="dialog"
        aria-label="Notifications"
        style={{
          position: "fixed",
          top: pos.top,
          right: pos.right,
          width: POPOVER_WIDTH,
          maxHeight: "var(--config-menu-max-height)",
          background: "var(--bg)",
          border: "var(--bw-hair) solid var(--line-strong)",
          borderRadius: "var(--r-3)",
          boxShadow: "var(--shadow-popover)",
          display: "flex",
          flexDirection: "column",
          zIndex: "var(--z-modal)" as unknown as number,
          fontFamily: "var(--font)",
          overflow: "hidden",
        }}
      >
        <PopoverHeader
          order={order}
          onToggleOrder={() =>
            setOrder((o) => (o === "newestFirst" ? "oldestFirst" : "newestFirst"))
          }
          onMarkAllRead={onMarkAllRead}
          onClear={() => setConfirmClear(true)}
          hasEntries={entries.length > 0}
        />
        <PopoverFilters
          kinds={kinds}
          onToggleKind={toggleKind}
          source={source}
          onChangeSource={setSource}
          windowKey={windowKey}
          onChangeWindow={setWindowKey}
          query={query}
          onChangeQuery={setQuery}
        />
        <div
          style={{
            flex: 1,
            minHeight: 0,
            overflowY: "auto",
            background: "var(--bg-sunken)",
          }}
        >
          {loading ? (
            <EmptyHint>Loading…</EmptyHint>
          ) : error ? (
            <EmptyHint danger>{error}</EmptyHint>
          ) : entries.length === 0 ? (
            <EmptyHint>
              {hasFilter(filter)
                ? "No matches. Adjust the filter or clear it."
                : "No notifications yet. Toasts and OS banners will collect here."}
            </EmptyHint>
          ) : (
            <ul
              style={{
                margin: 0,
                padding: 0,
                listStyle: "none",
              }}
            >
              {entries.map((e) => (
                <EntryRow
                  key={e.id}
                  entry={e}
                  onClick={() => onEntryClick(e)}
                />
              ))}
            </ul>
          )}
        </div>
      </div>
      {confirmClear && (
        <ConfirmDialog
          title="Clear notification history?"
          body={
            <p style={{ margin: 0 }}>
              All {entries.length} entries will be deleted. This can't be
              undone.
            </p>
          }
          confirmLabel="Clear"
          confirmDanger
          onCancel={() => setConfirmClear(false)}
          onConfirm={onClear}
        />
      )}
    </>
  );
}

function hasFilter(f: NotificationLogFilter): boolean {
  return (
    (f.kinds && f.kinds.length > 0) ||
    f.source !== undefined ||
    f.sinceMs !== undefined ||
    (f.query && f.query.length > 0) ||
    false
  );
}

// ── Header ───────────────────────────────────────────────────────

interface PopoverHeaderProps {
  order: NotificationLogOrder;
  onToggleOrder: () => void;
  onMarkAllRead: () => void;
  onClear: () => void;
  hasEntries: boolean;
}

function PopoverHeader({
  order,
  onToggleOrder,
  onMarkAllRead,
  onClear,
  hasEntries,
}: PopoverHeaderProps) {
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
        Notifications
      </h2>
      <button
        type="button"
        onClick={onToggleOrder}
        className="pm-focus"
        title={
          order === "newestFirst" ? "Sort oldest first" : "Sort newest first"
        }
        style={miniBtnStyle}
      >
        <Glyph
          g={order === "newestFirst" ? NF.chevronD : NF.chevronU}
          size="var(--fs-sm)"
        />
        <span>{order === "newestFirst" ? "Newest" : "Oldest"}</span>
      </button>
      <button
        type="button"
        onClick={onMarkAllRead}
        disabled={!hasEntries}
        className="pm-focus"
        title="Clear the unread badge"
        style={miniBtnStyle}
      >
        Mark read
      </button>
      <button
        type="button"
        onClick={onClear}
        disabled={!hasEntries}
        className="pm-focus"
        title="Delete every entry"
        style={miniBtnStyle}
      >
        Clear
      </button>
    </div>
  );
}

// ── Filter row ───────────────────────────────────────────────────

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

function PopoverFilters({
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

// ── Entry row ────────────────────────────────────────────────────

function EntryRow({
  entry,
  onClick,
}: {
  entry: NotificationEntry;
  onClick: () => void;
}) {
  const clickable = entry.target != null;
  const colors = kindColors(entry.kind);
  return (
    <li
      onClick={clickable ? onClick : undefined}
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
          width: 6,
          height: 6,
          marginTop: 6,
          borderRadius: "50%",
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
              fontSize: 10,
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
              marginTop: 2,
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
            marginTop: 3,
            fontSize: 10,
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

// ── Helpers ──────────────────────────────────────────────────────

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

function EmptyHint({
  children,
  danger,
}: {
  children: React.ReactNode;
  danger?: boolean;
}) {
  return (
    <div
      style={{
        padding: "var(--sp-24) var(--sp-16)",
        color: danger ? "var(--danger)" : "var(--fg-muted)",
        fontSize: "var(--fs-xs)",
        textAlign: "center",
      }}
    >
      {children}
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
  border: "var(--bw-subhair) solid var(--border)",
  borderRadius: "var(--r-1)",
  color: "var(--fg-muted)",
  cursor: "pointer",
  fontFamily: "inherit",
  lineHeight: 1.3,
};

const chipStyle: CSSProperties = {
  fontSize: 10,
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
  fontSize: 10,
  padding: "var(--sp-2) var(--sp-4)",
  border: "var(--bw-subhair) solid var(--border)",
  borderRadius: "var(--r-1)",
  background: "var(--bg-sunken)",
  color: "var(--fg-muted)",
  cursor: "pointer",
  fontFamily: "inherit",
};

// Reference Button so the bundler doesn't tree-shake the import we
// keep around for visual consistency in case future edits add a
// primary action footer. Cheap; not exported.
void Button;
