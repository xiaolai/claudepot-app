// Notification routing + persistent log API.
//
// Two surfaces share this module — they're cohesive because both end
// up under the same Tauri command file:
//
//   1. Click routing (`notificationActivateHostForSession`) — see the
//      detailed docstring below.
//   2. Notification log (`notificationLog*`) — the persistent
//      ring-buffer shown in the bell-icon popover. After the Phase 3
//      migration, the renderer-side path is exclusively
//      `notificationLogAppendRouted` (called by emit()); the legacy
//      `notificationLogAppend` lives on for pre-migration entries
//      reading off-disk and as a backward-compat IPC the Rust
//      service_status_watcher used to call directly.

import { invoke } from "@tauri-apps/api/core";
import type { NotificationTarget } from "../lib/notify";
import type {
  Category,
  Priority,
  Surface,
} from "../lib/notifications/types";

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
 *  opaque JSON value.
 *
 *  Phase 0 of the refactor added optional routing metadata —
 *  `category`, `priority`, `surfaces_requested`, `surfaces_delivered`.
 *  Pre-Phase-0 entries on disk carry only the legacy `source` field;
 *  post-Phase-1 entries (from `emit()`) populate the new fields and
 *  leave `source` null. Filters in the bell popover treat both shapes
 *  symmetrically. */
export interface NotificationEntry {
  id: number;
  ts_ms: number;
  /** Legacy surface tag, populated only on pre-Phase-0 entries.
   *  New entries from `emit()` carry surface info in
   *  `surfaces_requested` / `surfaces_delivered`. */
  source: NotificationSource | null;
  kind: NotificationKind;
  title: string;
  body: string;
  /** Renderer-defined click target; null when the surface had none. */
  target: NotificationTarget | null;
  /** Routing category, when the entry came through the `emit()`
   *  facade. `null` on legacy entries. */
  category: Category | null;
  /** Routing priority. `null` on legacy entries. */
  priority: Priority | null;
  /** Surfaces the dispatcher asked for BEFORE delivery gates fired. */
  surfaces_requested: Surface[];
  /** Surfaces that actually rendered. */
  surfaces_delivered: Surface[];
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

/** Args for the routed-emit IPC introduced in Phase 1. Mirrors the
 *  Rust DTO; the `emit()` facade is the only producer of these. */
export interface NotificationLogAppendRoutedArgs {
  category: Category;
  priority: Priority;
  kind: NotificationKind;
  title: string;
  body?: string;
  target?: NotificationTarget | null;
  surfacesRequested: Surface[];
  /** Surfaces already known-delivered at append time (toast + banner
   *  are always delivered if requested). OS-banner delivery is
   *  reported via `notificationLogMarkDelivered` after the OS
   *  dispatcher resolves. */
  surfacesDelivered: Surface[];
}

/** Category metadata returned by `notification_categories_metadata`.
 *  Drives the Settings → Notifications pane in Phase 4. */
export interface CategoryMeta {
  id: Category;
  label: string;
  group: string;
  priority: Priority;
  defaultEnabled: boolean;
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

  /** Phase 1: append an entry that came through the `emit()` facade.
   *  Records full routing metadata. Returns the assigned id so the
   *  caller can post `notificationLogMarkDelivered` after the OS
   *  dispatcher resolves. */
  notificationLogAppendRouted: (args: NotificationLogAppendRoutedArgs) =>
    invoke<number>("notification_log_append_routed", { args }),

  /** Phase 1: report OS-banner delivery after focus / permission /
   *  rate gates resolve. Idempotent. */
  notificationLogMarkDelivered: (id: number, surface: Surface) =>
    invoke<boolean>("notification_log_mark_delivered", { id, surface }),

  /** Phase 1: return the live category metadata table. The Settings
   *  pane mirrors Rust at runtime by reading this on mount. */
  notificationCategoriesMetadata: () =>
    invoke<CategoryMeta[]>("notification_categories_metadata"),
};
