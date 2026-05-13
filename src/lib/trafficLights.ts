/**
 * Runtime sync between the OS-placed traffic-light cluster and the
 * webview chrome. Reads the live `NSWindow.standardWindowButton`
 * frame from Rust, exposes it as CSS custom properties, and lets the
 * chrome's `transform: translateY(...)` formula pin its content onto
 * the OS centerline — not the chrome's geometric center.
 *
 * Why the runtime path: AppKit does not place the buttons at the
 * chrome's geometric center. The vertical offset between flex-
 * centered text and the actual button row depends on macOS version,
 * the button's reported height (14 / 16 px), the configured
 * `trafficLightPosition.y`, and AppKit's autoresizing during first
 * paint. Hardcoded magic numbers drift on every Tauri / macOS bump
 * (this repo's y went 14 → 21 → 22 across three months before this
 * fix). See `~/.claude/skills/tauri/SKILL.md` for the full rationale
 * and the matching Rust module at `src-tauri/src/traffic_light.rs`.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface TrafficLightMetrics {
  /** Center of the close button, logical CSS px, top-left origin. */
  center_x: number;
  center_y: number;
  /** Right edge of the close → zoom span in logical CSS px from the
   *  window's left edge. The chrome reads this to size its
   *  `--chrome-inset-left`. */
  right: number;
  width: number;
  height: number;
}

const isInTauri = (): boolean =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const apply = (m: TrafficLightMetrics): void => {
  const root = document.documentElement;
  root.style.setProperty("--traffic-light-center-y", `${m.center_y}px`);
  root.style.setProperty("--traffic-light-right", `${m.right}px`);
  // Override the static `--chrome-inset-left` token (88px design
  // value) with the actual right edge + a 12px breath so the
  // breadcrumb sits a consistent gap from whichever traffic-light
  // layout AppKit ended up producing.
  root.style.setProperty("--chrome-inset-left", `${m.right + 12}px`);
};

/**
 * Subscribe to the live metrics. Returns a teardown function — call
 * it from the same useEffect's cleanup so the listener doesn't leak
 * across hot-reloads.
 *
 * Outside Tauri (vitest, Storybook) returns a no-op teardown — the
 * chrome's CSS fallback to the chrome's geometric center keeps the
 * layout sensible.
 */
export async function installTrafficLightSync(): Promise<UnlistenFn> {
  if (!isInTauri()) return () => {};

  // Subscribe FIRST so the boot-time emit from Rust (300ms after
  // window creation, then again at 1300ms) doesn't race the
  // mount-time pull below.
  const unlisten = await listen<TrafficLightMetrics>(
    "traffic-light-metrics",
    (e) => apply(e.payload),
  );

  // Then pull once — the listener attaches after window-event
  // setup in some renders, and the boot emits may have already
  // fired by the time we get here.
  try {
    const initial = await invoke<TrafficLightMetrics | null>(
      "traffic_light_metrics",
    );
    if (initial) apply(initial);
  } catch {
    // Non-macOS or platforms where the NSWindow handle isn't
    // available return None; the CSS fallback applies.
  }

  return unlisten;
}
