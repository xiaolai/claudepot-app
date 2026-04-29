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
 *  Three independent gates apply in order:
 *    1. Focus gate: drop when `document.hasFocus()` (override with
 *       `ignoreFocus: true` for fatal-class alerts).
 *    2. Permission gate: probe → request-on-first-trigger → grant
 *       check.
 *    3. Coalescing gate: when `dedupeKey` is set, apply a per-key
 *       token bucket. Replaces the three different per-hook policies
 *       that used to live in useActivityNotifications (1-per-60s),
 *       useCardNotifications (3-in-60-then-summary), and the toast
 *       layer (no coalescing). Now one rule: max `maxBurst` per
 *       `windowMs` per `dedupeKey`.
 *
 *  Errors are swallowed — a denied permission must not interrupt
 *  the calling detection loop. */
export async function dispatchOsNotification(
  title: string,
  body: string,
  opts: DispatchOpts = {},
): Promise<boolean> {
  if (!opts.ignoreFocus && typeof document !== "undefined") {
    try {
      if (document.hasFocus()) return false;
    } catch {
      /* JSDOM without window — fall through and dispatch anyway */
    }
  }

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
}

/** Test-only: introspect bucket map size (catches leaks where unique
 *  per-event keys would otherwise grow without bound). */
export function __bucketsSizeForTests(): number {
  return buckets.size;
}
