// TS mirror of `crates/claudepot-core/src/notifications.rs`.
//
// Source-of-truth is Rust. This file is the hand-maintained type
// surface — runtime metadata (titles, descriptions, default state)
// comes from the `notification_categories_metadata` IPC at runtime,
// so adding a Category only requires updating:
//
//   1. The Rust `Category` enum + its `priority()` binding + its
//      `display_meta()` entry + the `all()` array.
//   2. The `Category` union below + the optional CATEGORY_NAMES
//      sweep for the runtime mirror test.
//
// A test in `src/lib/notifications/types.test.ts` (added in Phase 1)
// asserts the TS union matches the IPC-reported metadata so drift
// fails CI.

/**
 * Notification priority — the routing axis. Adding a new variant
 * requires a matching Rust `Priority` variant (see
 * `crates/claudepot-core/src/notifications.rs`).
 */
export type Priority = "p0Blocking" | "p1Stalled" | "p2Acknowledge" | "p3Ambient";

/**
 * Visual surfaces a routed event can target. Toast and Banner are
 * renderer-side; OsBanner is dispatched by the Tauri notification
 * plugin (Rust-side for events that originate in Rust, renderer-
 * side via the OS dispatcher otherwise).
 */
export type Surface = "toast" | "osBanner" | "banner";

/**
 * Source code's view of every notification category. Adding a
 * category requires the four lockstep changes documented in the
 * Rust module's header comment.
 *
 * Grouped here by default priority for skim-readability — the
 * actual priority binding lives in Rust.
 */
export type Category =
  // P0 — Blocking
  | "accountAuthRejected"
  | "keychainLocked"
  | "ccSlotDrift"
  | "desktopDrift"
  | "repairConflict"
  // P1 — Stalled
  | "sessionWaiting"
  | "sessionStuck"
  | "sessionErrorBurst"
  | "opDoneUnfocused"
  | "rotationSuggested"
  | "usageThreshold"
  | "updateInstallReady"
  // P2 — Acknowledge
  | "accountVerified"
  | "accountSwitched"
  | "projectRenamed"
  | "projectRepaired"
  | "sessionPruned"
  | "keyCopied"
  | "keyAdded"
  | "keyRemoved"
  | "configEdited"
  | "automationRan"
  | "rotationApplied"
  | "bannerResolved"
  // P3 — Ambient
  | "memoryChanged"
  | "configTreePatched"
  | "serviceStatusChanged"
  | "updateAvailable";

/**
 * Full enumeration used by the mirror-sweep test and any UI that
 * iterates every category (e.g. the Settings pane fallback if the
 * IPC is unavailable). Must match `Category::all()` in Rust.
 */
export const CATEGORY_NAMES: readonly Category[] = [
  "accountAuthRejected",
  "keychainLocked",
  "ccSlotDrift",
  "desktopDrift",
  "repairConflict",
  "sessionWaiting",
  "sessionStuck",
  "sessionErrorBurst",
  "opDoneUnfocused",
  "rotationSuggested",
  "usageThreshold",
  "updateInstallReady",
  "accountVerified",
  "accountSwitched",
  "projectRenamed",
  "projectRepaired",
  "sessionPruned",
  "keyCopied",
  "keyAdded",
  "keyRemoved",
  "configEdited",
  "automationRan",
  "rotationApplied",
  "bannerResolved",
  "memoryChanged",
  "configTreePatched",
  "serviceStatusChanged",
  "updateAvailable",
] as const;

/**
 * Surface set the dispatcher asks the primitives for, BEFORE any
 * delivery gates fire. Mirror of Rust `SurfaceSet`.
 */
export interface SurfaceSet {
  toast: boolean;
  osBanner: boolean;
  banner: boolean;
  log: boolean;
  ignoreFocus: boolean;
}

/**
 * Rotation orchestrator mode — drives the `RotationApplied` override
 * in routing. Today only `useRotationEvents` reads it; future
 * categories can plumb new fields into the same `DispatchContext`.
 */
export type RotationMode = "confirm" | "auto";

/**
 * Context the routing function reads when overriding priority
 * defaults. `window_focused` is the JS `document.hasFocus()` at
 * emit time; the OS dispatcher applies its own focus gate too, but
 * routing keeps the field available for future overrides.
 */
export interface DispatchContext {
  rotationMode?: RotationMode;
  windowFocused: boolean;
}

/**
 * Click target a notification carries through to the bell popover
 * and the OS-banner click-route queue. Imported from the existing
 * lib/notify module so the shape stays in one place; re-exported
 * here for convenience under the notifications namespace.
 */
export type { NotificationTarget } from "../notify";

/**
 * Severity tag — kept identical to today's `pushToast` shape plus
 * the existing OS `notice` tier. Logging defaults map cleanly:
 * P0/P1 errors → `error`, P0/P1 non-errors → `notice`, P2/P3 →
 * `info`. The emit() facade fills this from the event when the
 * caller doesn't override.
 */
export type NotificationKind = "info" | "notice" | "error";

/**
 * The minimum event payload an emit() caller supplies. The
 * dispatcher fills in priority (from category), surfaces_requested
 * (from route), surfaces_delivered (from primitive outcomes), and
 * logs once.
 */
export interface NotificationEvent {
  category: Category;
  /** Override the auto-derived kind when needed (e.g. an
   *  acknowledge-level event that should still render as error). */
  kind?: NotificationKind;
  title: string;
  body?: string;
  target?: import("../notify").NotificationTarget;
  /** Dedupe key for primitive-level coalescing. The dispatcher
   *  forwards it to the OS dispatcher's token-bucket. */
  dedupeKey?: string;
}

/**
 * Pure mirror of Rust `route()`. Keeps the policy in one place so
 * the TS emit() facade can preview surfaces synchronously without
 * an IPC round-trip — the Rust dispatcher applies the same rules
 * for events that originate Rust-side.
 *
 * Update both sides in lockstep when overrides change.
 */
export function route(
  event: { category: Category },
  ctx: DispatchContext,
): SurfaceSet {
  const priority = priorityForCategory(event.category);
  // eslint-disable-next-line prefer-const
  let s = surfaceSetForPriority(priority);
  // Category × context overrides — keep aligned with Rust.
  if (
    event.category === "rotationApplied" &&
    ctx.rotationMode === "auto"
  ) {
    s.toast = false;
  }
  return s;
}

export function priorityForCategory(category: Category): Priority {
  switch (category) {
    case "accountAuthRejected":
    case "keychainLocked":
    case "ccSlotDrift":
    case "desktopDrift":
    case "repairConflict":
      return "p0Blocking";
    case "sessionWaiting":
    case "sessionStuck":
    case "sessionErrorBurst":
    case "opDoneUnfocused":
    case "rotationSuggested":
    case "usageThreshold":
    case "updateInstallReady":
      return "p1Stalled";
    case "accountVerified":
    case "accountSwitched":
    case "projectRenamed":
    case "projectRepaired":
    case "sessionPruned":
    case "keyCopied":
    case "keyAdded":
    case "keyRemoved":
    case "configEdited":
    case "automationRan":
    case "rotationApplied":
    case "bannerResolved":
      return "p2Acknowledge";
    case "memoryChanged":
    case "configTreePatched":
    case "serviceStatusChanged":
    case "updateAvailable":
      return "p3Ambient";
  }
}

export function surfaceSetForPriority(p: Priority): SurfaceSet {
  switch (p) {
    case "p0Blocking":
      return { toast: false, osBanner: true, banner: true, log: true, ignoreFocus: true };
    case "p1Stalled":
      return { toast: false, osBanner: true, banner: false, log: true, ignoreFocus: false };
    case "p2Acknowledge":
      return { toast: true, osBanner: false, banner: false, log: true, ignoreFocus: false };
    case "p3Ambient":
      return { toast: false, osBanner: false, banner: false, log: true, ignoreFocus: false };
  }
}

/**
 * Translate a Rust `SurfaceSet` (from the routing IPC) into the list
 * of surfaces the dispatcher requested. Used by the bell popover to
 * render which surfaces were attempted.
 */
export function requestedSurfaces(s: SurfaceSet): Surface[] {
  const v: Surface[] = [];
  if (s.toast) v.push("toast");
  if (s.osBanner) v.push("osBanner");
  if (s.banner) v.push("banner");
  return v;
}
