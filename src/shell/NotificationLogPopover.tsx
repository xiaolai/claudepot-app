import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { api } from "../api";
import type {
  NotificationEntry,
  NotificationKind,
  NotificationLogFilter,
  NotificationLogOrder,
  NotificationSource,
} from "../api/notification";
import { NotificationLogFilters } from "./NotificationLogFilters";
import { NotificationLogEntry } from "./NotificationLogEntry";
import { NotificationLogHeader } from "./NotificationLogHeader";

/**
 * Anchored popover panel for the notification log. Mounted by
 * `NotificationBell` when the user clicks the chrome bell. Reads the
 * persistent ring buffer at `~/.claudepot/notifications.json` via
 * `api.notificationLogList`, with filter/sort/clear controls.
 *
 * Behavioral contracts:
 * - Mark-all-read fires on mount (popover open IS the "I'm looking
 *   at this" moment). The header button is a redundant affordance
 *   for users who close + reopen without scrolling.
 * - Outside-click + Escape both close.
 * - Filter state is local to one popover lifecycle — closing wipes
 *   it so the next open starts fresh.
 * - Click an entry with a non-null target → emit
 *   `claudepot:notification-log-target` so the App-shell focus
 *   router handles it the same way as a fresh banner click.
 *
 * The filter row + entry row live in their own files so this stays
 * under the per-file LOC limit and so each visual unit can be tested
 * in isolation.
 */

const POPOVER_WIDTH = 380;
const POPOVER_GAP = 8;

interface PopoverProps {
  anchorRef: React.RefObject<HTMLButtonElement | null>;
  onClose: () => void;
  onCountMaybeChanged: () => void;
}

export function NotificationLogPopover({
  anchorRef,
  onClose,
  onCountMaybeChanged,
}: PopoverProps) {
  const panelRef = useRef<HTMLDivElement | null>(null);
  const [entries, setEntries] = useState<NotificationEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);

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

  // Anchor by viewport-right so the panel never clips off-screen on a
  // narrow window. We push the popover in from the right edge by the
  // same offset as the bell's right edge so its right edge aligns
  // with the bell's right edge.
  const [pos, setPos] = useState<{ top: number; right: number } | null>(null);
  useEffect(() => {
    const compute = () => {
      const rect = anchorRef.current?.getBoundingClientRect();
      if (!rect) return;
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

  // Mark-all-read on open — the popover IS the user looking.
  useEffect(() => {
    void api
      .notificationLogMarkAllRead()
      .then(() => onCountMaybeChanged())
      .catch(() => {
        /* swallow */
      });
  }, [onCountMaybeChanged]);

  // Close on outside click. Mousedown so we close before any
  // down-stream click handlers fire (matches ContextMenu pattern).
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

  // Round-trip the entry's stored target through a window event so
  // the App-shell's focus router handles popover-clicks identically
  // to live banner-clicks. The router lives in App.tsx and listens
  // on `claudepot:notification-log-target`.
  const onEntryClick = useCallback(
    (entry: NotificationEntry) => {
      if (!entry.target) return;
      window.dispatchEvent(
        new CustomEvent("claudepot:notification-log-target", {
          detail: { target: entry.target },
        }),
      );
      onClose();
    },
    [onClose],
  );

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
        <NotificationLogHeader
          order={order}
          onToggleOrder={() =>
            setOrder((o) =>
              o === "newestFirst" ? "oldestFirst" : "newestFirst",
            )
          }
          onMarkAllRead={onMarkAllRead}
          onClear={() => setConfirmClear(true)}
          hasEntries={entries.length > 0}
        />
        <NotificationLogFilters
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
            <ul style={{ margin: 0, padding: 0, listStyle: "none" }}>
              {entries.map((e) => (
                <NotificationLogEntry
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
