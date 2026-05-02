// Shared OS-notification dispatcher.
//
// Three things this module owns that the hooks used to duplicate:
//
//   1. **Singleton permission probe.** One `isPermissionGranted()` call
//      on first use. One `requestPermission()` call lazily on first
//      dispatch. Hooks no longer race each other through their own
//      probe state machines.
//   2. **Window-focus gate.** OS notifications are noise when the
//      window already has focus — the in-app toast/banner/row already
//      shows the same signal. `dispatchOsNotification` returns silently
//      when `document.hasFocus()` is true. Pass `ignoreFocus: true` for
//      fatal-class alerts (auth-rejected, keychain-locked) where the
//      OS-level prominence is the point.
//   3. **Status subscribers.** Settings UI re-renders when the user
//      grants/denies permission via `requestNotificationPermission`,
//      without a polling interval.
//
// Non-Tauri / SSR safety: the dynamic plugin module isn't imported at
// the top level — we route through Vite's eager `import` to keep
// the test environment happy when `@tauri-apps/plugin-notification`
// is absent. Permission helpers fail closed.

import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { notificationApi } from "../api/notification";

/** Severity passed through to the notification log. Toasts pick
 *  `info` / `error`; OS dispatchers usually pick `notice` (non-error
 *  but prominent) or `error`. Defaults to `notice` when the caller
 *  doesn't specify, matching the OS-banner mental model — banners
 *  are never routine. */
export type OsNotificationKind = "info" | "notice" | "error";

export type PermissionStatus =
  | "unknown"
  | "not-requested"
  | "granted"
  | "denied";

let cached: PermissionStatus = "unknown";
const subscribers = new Set<(s: PermissionStatus) => void>();

function set(next: PermissionStatus) {
  if (cached === next) return;
  cached = next;
  for (const fn of subscribers) {
    try {
      fn(next);
    } catch {
      /* swallow — one bad subscriber must not poison the rest */
    }
  }
}

/** Read the current cached status synchronously. Triggers a probe on
 *  first call so callers don't need to await — the probe runs in the
 *  background and a subscriber will fire when it resolves. */
export function getPermissionStatus(): PermissionStatus {
  if (cached === "unknown") {
    void probe();
  }
  return cached;
}

/** Subscribe to status changes. Returns an unsubscribe callback. The
 *  subscriber is fired immediately with the current cached value, which
 *  may itself trigger the lazy probe — keeps the surface "render this,
 *  re-render on change" without forcing every consumer to thread an
 *  effect. */
export function subscribePermissionStatus(
  fn: (s: PermissionStatus) => void,
): () => void {
  subscribers.add(fn);
  fn(getPermissionStatus());
  return () => {
    subscribers.delete(fn);
  };
}

let probeInFlight: Promise<void> | null = null;

/** Internal: probe the OS for the current grant state. Idempotent —
 *  concurrent callers share the same in-flight promise. Result is
 *  pushed into `cached` and fanned out to subscribers.
 *
 *  Probe failure (the plugin throws) is treated as RETRYABLE — we
 *  fall back to `"unknown"` rather than `"denied"` so a transient
 *  Tauri-plugin glitch at startup doesn't permanently misclassify
 *  permission. The next caller of `getPermissionStatus`,
 *  `requestNotificationPermission`, or `dispatchOsNotification`
 *  will re-probe. */
async function probe(): Promise<void> {
  if (probeInFlight) return probeInFlight;
  probeInFlight = (async () => {
    try {
      const granted = await isPermissionGranted();
      // `isPermissionGranted` returns false BEFORE any prompt has
      // been shown — treat that as "not-requested" so first dispatch
      // can prompt rather than silently dropping forever.
      set(granted ? "granted" : "not-requested");
    } catch {
      // Plugin error (non-Tauri env, transient glitch). Stay
      // "unknown" so the next call re-probes; never cache "denied"
      // from a probe failure — only an explicit user denial via
      // `requestPermission` should land us in "denied".
      cached = "unknown";
    } finally {
      probeInFlight = null;
    }
  })();
  return probeInFlight;
}

/** Memoizes the in-flight `requestPermission()` call so concurrent
 *  callers (e.g. simultaneous first-dispatches from two hooks) share
 *  one OS prompt instead of racing two. Cleared once the promise
 *  settles. */
let requestInFlight: Promise<PermissionStatus> | null = null;

/** Explicitly request OS-notification permission. Resolves to the new
 *  cached state. Use from a button click — never from a passive
 *  effect, otherwise the OS prompt fires before the user expressed
 *  intent. The `dispatchOsNotification` helper does this lazily on
 *  the first actual dispatch, so hooks rarely need to call this. */
export async function requestNotificationPermission(): Promise<PermissionStatus> {
  if (requestInFlight) return requestInFlight;
  requestInFlight = (async () => {
    // Make sure we have a baseline so we don't double-prompt when
    // permission was already granted at startup.
    if (cached === "unknown") {
      await probe();
    }
    if (cached === "granted") return cached;
    try {
      const result = await requestPermission();
      set(result === "granted" ? "granted" : "denied");
    } catch {
      // Explicit prompt failure is more terminal than a probe
      // failure — the user either denied or the plugin can't
      // surface a prompt. Either way, classify as "denied" so we
      // don't loop. The Settings UI can offer a manual "Request"
      // button if the user wants to retry from System Settings.
      set("denied");
    }
    return cached;
  })();
  try {
    return await requestInFlight;
  } finally {
    requestInFlight = null;
  }
}

/**
 * What the user wants to do when they click a notification's banner.
 *
 * `host` — focus the terminal/editor running the session. The
 *   App-shell consumer translates this to a Tauri command that walks
 *   the session's process tree and activates the host app. Falls
 *   back to `app:projects:<sid>` when the host can't be resolved.
 *
 * `app` — focus Claudepot, deep-linked to a specific surface. The
 *   `route` field is one of the known internal nav targets (a
 *   section id, optionally with a sub-route).
 *
 * `info` — purely informational; click is a no-op (focus alone is
 *   the action). Reserved for tray-only signals where there's no
 *   meaningful "where to go."
 *
 * The discriminator is intentionally narrow: every Claudepot
 * notification site declares its intent at dispatch time; the
 * shell-level consumer routes without further interpretation.
 */
export type NotificationTarget =
  | { kind: "host"; session_id: string; cwd: string }
  | {
      kind: "app";
      route:
        | { section: "projects"; session_id?: string; cwd?: string }
        | { section: "accounts"; email?: string }
        | { section: "settings"; sub?: string }
        | { section: "events" };
    }
  | { kind: "info" };

interface DispatchOpts {
  /** Bypass the `document.hasFocus()` gate. Use only for fatal-class
   *  alerts where the OS-level prominence is the point (auth rejected,
   *  keychain locked). Default false: in-app channels handle the
   *  focused case. */
  ignoreFocus?: boolean;
  /** Coalescing key. When set, the dispatcher applies a token bucket
   *  per key: at most `maxBurst` notifications per `windowMs`. Beyond
   *  that, dispatches are silently dropped (return false). Defaults
   *  match the old card-coalescing policy. Use distinct keys for
   *  unrelated notifications (`session:<sid>`, `card:<kind>:<title>`,
   *  `op:<op_id>`) so one busy session can't starve another. */
  dedupeKey?: string;
  /** Max notifications per `windowMs` window. Default 3. */
  maxBurst?: number;
  /** Token-bucket window in milliseconds. Default 60_000. */
  windowMs?: number;
  /** OS-side notification grouping. macOS reads this as `threadId`
   *  so related notifications stack into one expandable banner.
   *  Linux libnotify ignores it. Windows toasts read it as `group`.
   *  Pass a stable key per logical conversation — usually a project
   *  cwd or an op kind. */
  group?: string;
  /** Sound option passed through to the OS. Default omitted (silent
   *  on Linux/Windows, default chime on macOS). Pass "default" to
   *  force the OS default sound. */
  sound?: string;
  /** Where the user goes when they click this notification's banner.
   *
   *  The Tauri 2 desktop notification plugin (v2.3.3) does NOT wire
   *  body-click events back to JS — `onAction` and
   *  `registerActionTypes` are mobile-only. Verified by reading the
   *  plugin source at `tauri-plugin-notification/src/desktop.rs`,
   *  which spawns `notify_rust::Notification::show()` and discards
   *  the handle.
   *
   *  Workaround: when an OS notification is actually dispatched, we
   *  push the target onto a small in-memory queue with the dispatch
   *  timestamp. The App-shell focus listener consumes the most
   *  recent unexpired entry whenever the window gains focus. The
   *  10-second window keeps false-positives bounded — a user who
   *  ignores a notification and opens Claudepot manually 11 seconds
   *  later doesn't get routed to a stale destination.
   *
   *  Targets are recorded only for accepted dispatches (not for
   *  focus-gated, permission-denied, or rate-limited drops), so
   *  the queue size is bounded by the rate-limit policy itself. */
  target?: NotificationTarget;
  /** Severity recorded in the notification log. Doesn't affect OS
   *  dispatch (the OS picks the visual). Defaults to `notice` —
   *  banners are by definition not routine. */
  kind?: OsNotificationKind;
}

// Token-bucket state: per-key list of recent dispatch timestamps
// alongside the bucket's own `windowMs`. Storing window per-bucket
// lets the sweep evict each entry by its own deadline — required so
// unique-per-event keys (e.g. `op:<uuid>` from useOpDoneNotifications)
// can use a short window without depending on the longest live
// window in the map. Without this, the sweep would either over- or
// under-evict whenever windows differ across keys.
interface Bucket {
  stamps: number[];
  windowMs: number;
}
const buckets = new Map<string, Bucket>();

const DEFAULT_BURST = 3;
const DEFAULT_WINDOW_MS = 60_000;

/** How long a dispatched click target stays consumable. macOS surfaces
 *  banners for ~5 s by default; we double that to absorb user latency
 *  (slide cursor up, click). After this window the target expires —
 *  if the user opens Claudepot from the dock or tray later, they
 *  shouldn't get routed to a stale destination. */
const TARGET_TTL_MS = 10_000;

/** Click-target queue. One entry per dispatched notification that
 *  declared an intent. `consumeRecentTarget()` pops the most-recent
 *  unexpired entry; older entries are evicted on each push and on
 *  each consumption attempt.
 *
 *  Bounded implicitly by the rate-limit policy — the dispatcher only
 *  pushes when a notification was actually accepted, and per-key
 *  rate limits keep accept rate to single-digit per minute under
 *  normal conditions. The 32-entry hard cap below is a defense
 *  against a future bug that bypasses dedupe; never expected to
 *  trip in practice. */
interface QueuedTarget {
  target: NotificationTarget;
  dispatched_at: number;
}
const targetQueue: QueuedTarget[] = [];
const TARGET_QUEUE_HARD_CAP = 32;

function pushTarget(target: NotificationTarget, now: number): void {
  // Evict expired entries on every push so the queue stays small
  // even when consumers never call consumeRecentTarget.
  while (targetQueue.length > 0 && now - targetQueue[0].dispatched_at > TARGET_TTL_MS) {
    targetQueue.shift();
  }
  targetQueue.push({ target, dispatched_at: now });
  while (targetQueue.length > TARGET_QUEUE_HARD_CAP) {
    targetQueue.shift();
  }
}

/** Pop the most-recent unexpired target. Returns `null` when the
 *  queue is empty or every entry has expired. The "most recent"
 *  policy matches user intent: when the user clicks a banner, the
 *  banner that most recently appeared is what they're acting on. */
export function consumeRecentTarget(): NotificationTarget | null {
  const now = Date.now();
  while (targetQueue.length > 0) {
    const entry = targetQueue.pop();
    if (!entry) return null;
    if (now - entry.dispatched_at <= TARGET_TTL_MS) {
      // Drop everything older than the consumed entry — they belong
      // to earlier banners the user dismissed by acting on this one.
      targetQueue.length = 0;
      return entry.target;
    }
    // Expired; loop continues to try the next-most-recent.
  }
  return null;
}

/** Sweep every bucket: drop expired stamps, delete fully-empty
 *  entries. Called from `dispatchOsNotification` so eviction stays
 *  proportional to traffic. The cost is `O(buckets)` per dispatch,
 *  which is fine for the typical < 50 simultaneously-active keys. */
function sweepBuckets(now: number): void {
  for (const [key, bucket] of buckets) {
    const live = bucket.stamps.filter((t) => now - t < bucket.windowMs);
    if (live.length === 0) {
      buckets.delete(key);
    } else if (live.length !== bucket.stamps.length) {
      buckets.set(key, { stamps: live, windowMs: bucket.windowMs });
    }
  }
}

/** Test-only: clear bucket state. */
function resetBuckets(): void {
  buckets.clear();
}

/** Fire an OS notification, lazily prompting for permission if the
 *  user hasn't been asked yet. Returns true if the OS sink received
 *  the call, false if the dispatch was suppressed (focused window,
 *  denied permission, no Tauri, rate-limited).
 *
 *  Three independent gates apply to the OS-banner pipeline:
 *    1. Rate-limit gate: when `dedupeKey` is set, apply a per-key
 *       token bucket. Max `maxBurst` per `windowMs` per `dedupeKey`.
 *    2. Permission gate: probe → request-on-first-trigger → grant
 *       check.
 *    3. Focus gate: skip the OS banner when `document.hasFocus()`
 *       (override with `ignoreFocus: true` for fatal-class alerts).
 *
 *  **The notification log writes BEFORE the OS-banner gates.** A
 *  rate-limited entry still records the intent (so the bell shows
 *  the burst even when the OS center is correctly suppressed); a
 *  focus-gated entry still records (so the user, who saw nothing,
 *  can find what they were notified-about-but-didn't-see). The log
 *  records what Claudepot WANTED to surface; OS delivery is a
 *  separate concern with its own gates.
 *
 *  The return value reflects the OS-banner outcome only — `true` if
 *  the OS notification fired, `false` if any gate suppressed it.
 *  Callers that care about delivery (none today) read the boolean;
 *  the log records intent regardless.
 *
 *  Errors are swallowed — a denied permission must not interrupt
 *  the calling detection loop. */
export async function dispatchOsNotification(
  title: string,
  body: string,
  opts: DispatchOpts = {},
): Promise<boolean> {
  // ── Step 1: ALWAYS log the intent to notify ──────────────────
  //
  // Fire-and-forget IPC. The Rust side is no-op-safe when the file
  // open failed at boot (see lib.rs fallback path). We write the
  // log entry before any OS-banner gating so:
  //
  //   - Focus-gated dispatches (window focused, banner suppressed)
  //     still surface in the bell. Pre-fix, focus-gated usage-
  //     threshold notifications were silently lost — no toast, no
  //     banner, no log entry. Now the bell catches them.
  //   - Rate-limited dispatches (token-bucket cap reached) still
  //     log so the user can see how many crossings happened even
  //     when the OS center was correctly suppressing the noise.
  //   - Permission-denied dispatches log too — the user opted out
  //     of OS banners, not out of in-app history. The log is the
  //     in-app surface; gating it on permission would conflate two
  //     opt-ins.
  void notificationApi
    .notificationLogAppend({
      source: "os",
      kind: opts.kind ?? "notice",
      title,
      body,
      target: opts.target ?? null,
    })
    .then(() => {
      window.dispatchEvent(new Event("claudepot:notification-logged"));
    })
    .catch(() => {
      /* swallow — log persistence is advisory, never load-bearing */
    });

  // ── Step 2: OS-banner pipeline ───────────────────────────────

  // Sweep expired buckets on EVERY dispatch (deduped or not), so
  // single-shot keys like `op:<uuid>` don't linger until the next
  // deduped dispatch. Without this, a burst of unique-key dispatches
  // followed by silence would leak one entry per dispatch until the
  // next deduped call.
  const now = Date.now();
  if (buckets.size > 0) sweepBuckets(now);

  // Token-bucket gate. Inserted BEFORE the permission probe so a
  // burst of denied dispatches doesn't repeatedly enter the prompt
  // path. Bucket entries are timestamps; we filter expired ones,
  // check the cap, and append on accept.
  if (opts.dedupeKey) {
    const cap = opts.maxBurst ?? DEFAULT_BURST;
    const windowMs = opts.windowMs ?? DEFAULT_WINDOW_MS;
    const existing = buckets.get(opts.dedupeKey);
    const stamps = (existing?.stamps ?? []).filter(
      (t) => now - t < windowMs,
    );
    if (stamps.length >= cap) {
      // Update the bucket to the trimmed list so expired stamps
      // don't linger forever in the map.
      buckets.set(opts.dedupeKey, { stamps, windowMs });
      return false;
    }
    stamps.push(now);
    buckets.set(opts.dedupeKey, { stamps, windowMs });
  }

  // Focus gate — moved AFTER the log write but before the permission
  // probe. Focused window means we don't need an OS banner; the user
  // is already looking, the bell badge just incremented, the log
  // popover is one click away. Pass `ignoreFocus: true` for fatal-
  // class alerts (auth rejected, keychain locked) where the OS-level
  // prominence is the point.
  if (!opts.ignoreFocus && typeof document !== "undefined") {
    try {
      if (document.hasFocus()) return false;
    } catch {
      /* JSDOM without window — fall through and dispatch anyway */
    }
  }

  if (cached === "unknown") {
    await probe();
  }

  if (cached === "not-requested") {
    // First dispatch since the user opted in — ask now. If they
    // grant, send this alert too. If they deny, drop silently.
    await requestNotificationPermission();
  }

  if (cached !== "granted") return false;

  try {
    // Pass through OS metadata when present. Tauri 2's notification
    // plugin forwards `group` to macOS as `threadId` (banners stack)
    // and to Windows toasts as the group attribute; Linux libnotify
    // ignores it. `sound: "default"` triggers the system chime on
    // macOS and is otherwise harmless.
    const payload: {
      title: string;
      body: string;
      group?: string;
      sound?: string;
    } = { title, body };
    if (opts.group) payload.group = opts.group;
    if (opts.sound) payload.sound = opts.sound;
    sendNotification(payload);
    // Record the click target only for accepted OS dispatches. The
    // App-shell focus listener consumes the most-recent unexpired
    // entry on `window` focus. See the `target` field's docstring
    // for the rationale. (Bell-popover clicks consume the entry's
    // own stored target through a separate event, not this queue.)
    if (opts.target) pushTarget(opts.target, now);
    return true;
  } catch {
    return false;
  }
}

/** Test-only: reset the cached state. Vitest needs this between tests
 *  because the singleton outlives any one render tree. Not exported
 *  from `index.ts` — consumers should use the public API. */
export function __resetForTests(): void {
  cached = "unknown";
  subscribers.clear();
  probeInFlight = null;
  requestInFlight = null;
  resetBuckets();
  targetQueue.length = 0;
}

/** Test-only: introspect the click-target queue size. Catches leaks
 *  from a future bug that pushes targets without the rate-limit gate
 *  in front of them. */
export function __targetQueueSizeForTests(): number {
  return targetQueue.length;
}

/** Test-only: introspect bucket map size (catches leaks where unique
 *  per-event keys would otherwise grow without bound). */
export function __bucketsSizeForTests(): number {
  return buckets.size;
}
