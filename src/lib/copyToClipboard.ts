/**
 * Copy text to the clipboard, surfacing both success and failure as
 * toasts. The native `navigator.clipboard.writeText` rejects in
 * narrow but real cases (no document focus, HTTPS-only contexts in
 * some browsers, Tauri webview policies) — silently toasting success
 * lies to the user, so we route both outcomes through the same
 * setter the call site already uses.
 *
 * Audit T4-8: `navigator.clipboard.writeText` can also throw
 * synchronously when the API surface is missing entirely (older
 * webviews, SSR contexts, narrow Tauri configs). The previous
 * `void navigator.clipboard.writeText(...).then(...)` chain assumed
 * the call always returned a Promise — when it threw before
 * returning, the synchronous exception propagated past the .then()
 * branches and crashed the caller before any toast could land.
 *
 * Fix: guard the API surface, wrap the call in try/catch, and
 * resolve to a boolean so callers can branch on success when needed.
 * The promise still resolves rather than rejects in failure cases
 * to preserve the existing fire-and-forget call pattern.
 */
export async function copyToClipboard(
  text: string,
  label: string,
  setToast: (msg: string) => void,
): Promise<boolean> {
  const writeText = navigator.clipboard?.writeText;
  if (typeof writeText !== "function") {
    setToast(`Couldn't copy ${label}: clipboard API unavailable.`);
    return false;
  }
  try {
    // Bind explicitly — pulled-out method references lose `this`
    // on some engines, which would itself throw synchronously.
    await navigator.clipboard.writeText(text);
    setToast(`Copied ${label}.`);
    return true;
  } catch (e) {
    const reason = e instanceof Error ? e.message : String(e);
    setToast(`Couldn't copy ${label}: ${reason}`);
    return false;
  }
}
