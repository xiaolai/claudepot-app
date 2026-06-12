/**
 * Centralized `requestIdleCallback` shim. WebKit (Tauri on macOS)
 * still doesn't expose rIC, so every caller needs a `setTimeout`
 * fallback — this module is the one home for that pattern (it used
 * to be copy-pasted across App.tsx, useSection, useRunningOps,
 * usePendingJournals, and useRefresh).
 *
 * `fallbackDelayMs` only applies when rIC is missing. Callers that
 * used `setTimeout(cb, 250)` to approximate "one idle slot" keep
 * that value; callers that used `0` take the default.
 */

export function requestIdle(
  cb: () => void,
  opts?: { fallbackDelayMs?: number },
): number {
  const w = window as typeof window & {
    requestIdleCallback?: (cb: () => void) => number;
  };
  if (typeof w.requestIdleCallback === "function") {
    return w.requestIdleCallback(cb);
  }
  return window.setTimeout(cb, opts?.fallbackDelayMs ?? 0);
}

export function cancelIdle(handle: number): void {
  const w = window as typeof window & {
    cancelIdleCallback?: (h: number) => void;
  };
  if (typeof w.cancelIdleCallback === "function") {
    w.cancelIdleCallback(handle);
  } else {
    window.clearTimeout(handle);
  }
}
