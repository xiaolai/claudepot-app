// Notification routing + persistent log API.
//
// Two surfaces share this module — they're cohesive because both end
// up under the same Tauri command file:
//
//   1. Click routing (`notificationActivateHostForSession`) — see the
//      detailed docstring below.
//   2. Notification log (`notificationLog*`) — the persistent
//      ring-buffer shown in the bell-icon popover. Every
//      `dispatchOsNotification` and `pushToast` call appends here.

import { invoke } from "@tauri-apps/api/core";
import type { NotificationTarget } from "../lib/notify";

/** Origin surface of a logged notification. Toasts and OS banners are
 *  dispatched independently — focus state determines which one(s)
 *  fire — so the log records each delivery separately. The popover's
 *  Source filter exposes both axes. */
export type NotificationSource = "toast" | "os";

/** User-facing severity. `notice` is OS-only territory today —
 *  reserved for non-error signals that warrant prominence (auth
 *  rejected, long-running op completion). Toasts coerce their `info`
 *  / `error` kind 1:1. */
export type NotificationKind = "info" | "notice" | "error";

/** One persisted entry. `id` and `tsMs` are assigned server-side so
 *  the renderer cannot mis-order itself. `target` is the renderer's
 *  own `NotificationTarget` shape, round-tripped through Rust as an
 *  opaque JSON value. */
export interface NotificationEntry {
  id: number;
  ts_ms: number;
  source: NotificationSource;
  kind: NotificationKind;
  title: string;
  body: string;
  /** Renderer-defined click target; null when the surface had none. */
  target: NotificationTarget | null;
}

/** Filter shape mirrored from `claudepot_core::notification_log`.
 *  Empty `kinds` = any kind; missing `source` = both surfaces;
 *  missing `sinceMs` = no time floor; empty `query` = no text
 *  filter. */
export interface NotificationLogFilter {
  kinds?: NotificationKind[];
  source?: NotificationSource;
  sinceMs?: number;
  query?: string;
}

export type NotificationLogOrder = "newestFirst" | "oldestFirst";

export interface NotificationLogAppendArgs {
  source: NotificationSource;
  kind: NotificationKind;
  title: string;
  body?: string;
  target?: NotificationTarget | null;
}

export const notificationApi = {
  /**
   * Activate the host terminal/editor running the given live
   * session. The backend looks up the session's PID via the live
   * runtime, walks parent processes to the first known terminal/
   * editor bundle, and asks LaunchServices to bring it to the
   * foreground. Returns `true` when a host was activated, `false`
   * when none could be resolved (session ended, or its host
   * process is unknown to us). `false` is the renderer's signal
   * to fall back to deep-linking the transcript inside Claudepot.
   *
   * Best-effort by design: there's no guarantee the host process
   * is still alive at click time, no guarantee the multiplexer's
   * pane can be focused, no guarantee an SSH'd remote session
   * has any local GUI host at all. The renderer's fallback path
   * handles all three.
   */
  notificationActivateHostForSession: (sessionId: string) =>
    invoke<boolean>("notification_activate_host_for_session", { sessionId }),

  /** Append a single entry. Call sites in `lib/notify.ts` and
   *  `hooks/useToasts.ts` fire this and discard the result — the log
   *  is best-effort and must never block the dispatch path. */
  notificationLogAppend: (args: NotificationLogAppendArgs) =>
    invoke<number>("notification_log_append", { args }),

  /** List entries matching `filter`, in `order`, capped at `limit`
   *  (or the buffer cap). The popover passes the user's filter +
   *  sort selections through verbatim. */
  notificationLogList: (
    filter?: NotificationLogFilter,
    order?: NotificationLogOrder,
    limit?: number,
  ) =>
    invoke<NotificationEntry[]>("notification_log_list", {
      filter,
      order,
      limit,
    }),

  /** Mark every current entry as seen. The bell badge clears until a
   *  fresh entry lands. */
  notificationLogMarkAllRead: () =>
    invoke<void>("notification_log_mark_all_read"),

  /** Wipe every entry and reset the id counter. */
  notificationLogClear: () => invoke<void>("notification_log_clear"),

  /** Current unread count. Drives the bell badge. */
  notificationLogUnreadCount: () =>
    invoke<number>("notification_log_unread_count"),
};
