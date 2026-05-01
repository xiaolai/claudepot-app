import { useCallback, useEffect, useRef, useState } from "react";
import type { MouseEvent } from "react";
import { IconButton } from "../components/primitives/IconButton";
import { NF } from "../icons";
import { api } from "../api";
import { NotificationLogPopover } from "./NotificationLogPopover";

/**
 * Bell icon with unread badge. Sits in `WindowChrome` between the
 * palette hint and the theme toggle. Click → mounts the popover
 * panel anchored beneath the bell.
 *
 * Why bell-in-WindowChrome rather than a top-level section: the
 * notification log is meta-UI ("what did this app just tell me?"),
 * not a domain noun (account/cli/desktop/project). Adding a section
 * would inflate the primary nav for content that's read-only and
 * bursty. A chrome icon with an unread badge — the standard surface
 * for this in every desktop app since Mac OS X 10.5 — fits the
 * actual cognitive load.
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
            unread > 0 ? `Notifications (${unread} unread)` : "Notifications"
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
              top: "var(--sp-2)",
              right: "var(--sp-2)",
              minWidth: "var(--sp-12)",
              height: "var(--sp-12)",
              padding: "0 var(--sp-2)",
              borderRadius: "var(--r-pill)",
              background: "var(--accent)",
              color: "var(--accent-text)",
              fontSize: "var(--fs-3xs)",
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
