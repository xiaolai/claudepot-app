// Phase 1 — the single `emit()` facade.
//
// One entry point. One log row per logical event. The facade
// computes routing, fans out to primitives with `_suppressLog: true`,
// records the routed log entry once, and posts back delivery
// outcomes after primitives finish.
//
// Migration shape: existing call sites keep working unchanged
// because `pushToast` and `dispatchOsNotification` still log when
// called without `_suppressLog`. The migration is opt-in per call
// site — convert each `pushToast(...)` to `emit({...})` and the
// double-log goes away for that site. Phase 3 strips the
// `_suppressLog` shim once the codebase is fully on `emit()`.
//
// CategoryPrefs reading is wired in Phase 1.5 — for now `emit()`
// always routes per `route()`. After 1.5 lands, a muted category
// will yield `surfaces_requested = []` and a log-only row.

import { dispatchOsNotification, type DispatchOpts } from "../notify";
import type {
  Category,
  DispatchContext,
  NotificationEvent,
  NotificationKind,
  Surface,
  SurfaceSet,
} from "./types";
import {
  priorityForCategory,
  requestedSurfaces,
  route,
} from "./types";
import { notificationApi } from "../../api/notification";
import { getCategoryPref } from "./prefs";
import type { CategoryPrefs } from "../../api/settings";

// ─── Toast primitive surface ──────────────────────────────────────
//
// The toast hook returns its push function from a context; we receive
// it through `buildEmit`'s deps so the facade stays a free function
// that can be tested without a React tree.

/** Toast push function shape — matches `useToasts.pushToast`. */
export type ToastPushFn = (
  kind: "info" | "error",
  text: string,
  onUndo?: () => void,
  opts?: {
    undoMs?: number;
    durationMs?: number;
    undoLabel?: string;
    onCommit?: () => void;
    dedupeKey?: string;
    _suppressLog?: boolean;
  },
) => void;

/** OS-banner dispatcher shape — matches `dispatchOsNotification`. */
export type OsDispatchFn = (
  title: string,
  body: string,
  opts?: DispatchOpts,
) => Promise<boolean>;

/** Notification log append shape — matches the IPC client method. */
export type LogAppendRoutedFn = typeof notificationApi.notificationLogAppendRouted;
/** Mark-delivered IPC shape. */
export type LogMarkDeliveredFn = typeof notificationApi.notificationLogMarkDelivered;

// ─── Dependencies the facade reads ─────────────────────────────────

export interface EmitDeps {
  /** Live `pushToast` from `useToasts`. */
  pushToast: ToastPushFn;
  /** OS dispatcher. Defaults to the singleton in `lib/notify.ts`. */
  dispatchOs?: OsDispatchFn;
  /** Log-append IPC. Defaults to the real `notificationApi` method.
   *  Tests inject a stub. */
  logAppendRouted?: LogAppendRoutedFn;
  /** Mark-delivered IPC. Defaults to the real method. */
  logMarkDelivered?: LogMarkDeliveredFn;
  /** Provider of the current dispatch context. Defaults to a focus-
   *  reading impl. Override in tests / when rotation mode matters. */
  getContext?: () => DispatchContext;
  /** Pref reader. Defaults to the renderer-side cache in `prefs.ts`.
   *  Tests inject a stub to isolate the dispatch invariants from
   *  the cache hydration state. */
  getPref?: (category: Category) => CategoryPrefs;
}

/** What `emit()` returns to the caller — mostly for tests, since
 *  every consumer today is fire-and-forget. */
export interface EmitResult {
  /** The log entry id when persistence succeeded. `null` when the
   *  log IPC errored (advisory; never blocks the in-app surface). */
  logId: number | null;
  surfaces: SurfaceSet;
  delivered: Surface[];
}

/** A bound emit dispatcher. Returned by `buildEmit`. */
export type EmitFn = (event: NotificationEvent) => Promise<EmitResult>;

// ─── Severity heuristic ────────────────────────────────────────────
//
// `NotificationKind` is the existing severity tag the bell popover
// reads. The dispatcher derives it from priority unless the caller
// overrides — keeps emit() call sites short.

function defaultKindForPriority(p: ReturnType<typeof priorityForCategory>): NotificationKind {
  switch (p) {
    case "p0Blocking":
      return "error";
    case "p1Stalled":
      return "notice";
    case "p2Acknowledge":
    case "p3Ambient":
      return "info";
  }
}

// ─── Toast text formatter ──────────────────────────────────────────
//
// Existing toasts use one inline string ("Renamed foo → bar"). The
// `emit()` shape has structured title + body so we get the bell
// popover's two-line layout. For the toast surface we collapse to a
// single line: title only if body is empty, else "title — body".

function toastText(event: NotificationEvent): string {
  if (!event.body || event.body.trim() === "") return event.title;
  return `${event.title} — ${event.body}`;
}

// ─── Facade constructor ────────────────────────────────────────────

/** Build an `emit()` dispatcher bound to the given deps. The
 *  AppStateProvider builds one per renderer instance using its live
 *  `pushToast` and the singleton OS dispatcher. */
export function buildEmit(deps: EmitDeps): EmitFn {
  const dispatchOs = deps.dispatchOs ?? dispatchOsNotification;
  const logAppend =
    deps.logAppendRouted ?? notificationApi.notificationLogAppendRouted;
  const logMark =
    deps.logMarkDelivered ?? notificationApi.notificationLogMarkDelivered;
  const getCtx =
    deps.getContext ??
    ((): DispatchContext => ({
      windowFocused:
        typeof document !== "undefined" ? document.hasFocus() : false,
    }));
  const getPref = deps.getPref ?? getCategoryPref;

  return async (event: NotificationEvent): Promise<EmitResult> => {
    const ctx = getCtx();
    const priority = priorityForCategory(event.category);
    const routed = route(event, ctx);
    const kind = event.kind ?? defaultKindForPriority(priority);

    // Apply user prefs to the routed set. `enabled: false` mutes
    // every surface (log still lands so the bell records a
    // forensic trail of suppressed events — the audit's intent
    // recording principle). `osOverride` forces the OS surface
    // on or off regardless of category default.
    const pref = getPref(event.category);
    const surfaces: SurfaceSet = pref.enabled
      ? {
          ...routed,
          osBanner:
            pref.osOverride === null
              ? routed.osBanner
              : pref.osOverride,
        }
      : { toast: false, osBanner: false, banner: false, log: true, ignoreFocus: false };

    // Surfaces toast + banner are renderer-side and always deliver
    // when requested (banner Phase 5 adds the state-machine layer).
    // OS-banner delivery is reported back after `dispatchOs` resolves.
    const delivered: Surface[] = [];
    if (surfaces.toast) delivered.push("toast");
    if (surfaces.banner) delivered.push("banner");

    // 1. Append the routed log entry. Capture the id before fanning
    //    out so we can post mark_delivered after the OS dispatcher
    //    resolves. Log writes are advisory — failure here must not
    //    block dispatch.
    let logId: number | null = null;
    try {
      logId = await logAppend({
        category: event.category,
        priority,
        kind,
        title: event.title,
        body: event.body ?? "",
        target: event.target ?? null,
        surfacesRequested: requestedSurfaces(surfaces),
        surfacesDelivered: delivered,
      });
      // Same-process signal so the bell badge updates without
      // waiting for the 8 s poll. Matches the legacy
      // notificationLogAppend's behavior.
      if (typeof window !== "undefined") {
        window.dispatchEvent(new Event("claudepot:notification-logged"));
      }
    } catch {
      /* swallow — log persistence is advisory */
    }

    // 2. Toast surface (synchronous push into the in-app queue).
    if (surfaces.toast) {
      const action = event.toastAction;
      deps.pushToast(
        kind === "error" ? "error" : "info",
        toastText(event),
        action?.onPress,
        {
          dedupeKey: event.dedupeKey,
          undoLabel: action?.label,
          undoMs: action?.timeoutMs,
          onCommit: action?.onCommit,
          _suppressLog: true,
        },
      );
    }

    // 3. OS-banner surface — async with delivery gates.
    if (surfaces.osBanner) {
      try {
        const fired = await dispatchOs(event.title, event.body ?? "", {
          ignoreFocus: surfaces.ignoreFocus,
          dedupeKey: event.dedupeKey,
          target: event.target,
          kind: kind === "info" ? "info" : kind === "error" ? "error" : "notice",
          _suppressLog: true,
        });
        if (fired) {
          delivered.push("osBanner");
          if (logId !== null) {
            // Best-effort; ignore failures (entry may have evicted
            // out of the ring buffer under burst).
            try {
              await logMark(logId, "osBanner");
            } catch {
              /* swallow */
            }
          }
        }
      } catch {
        /* swallow — OS dispatch failures don't propagate */
      }
    }

    // 4. Banner surface — Phase 5 plugs the state machine here. For
    //    now no-op: `useStatusIssues` still derives banners from app
    //    state; the `BannerResolved` paired event lands when 5 ships.

    return { logId, surfaces, delivered };
  };
}

// ─── React glue ────────────────────────────────────────────────────
//
// A thin hook so call sites consume `useEmit()` without threading
// deps. AppStateProvider builds the dispatcher and exposes it via
// context (wired in this same phase). Tests can build their own and
// inject by passing deps directly.

export type { Category, NotificationEvent };
