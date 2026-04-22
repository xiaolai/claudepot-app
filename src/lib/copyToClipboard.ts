/**
 * Copy text to the clipboard, surfacing both success and failure as
 * toasts. The native `navigator.clipboard.writeText` rejects in
 * narrow but real cases (no document focus, HTTPS-only contexts in
 * some browsers, Tauri webview policies) — silently toasting success
 * lies to the user, so we route both outcomes through the same
 * setter the call site already uses.
 *
 * The promise is intentionally fire-and-forget (`void`) so call
 * sites stay synchronous, but the failure path still lands a user-
 * visible message rather than going to a swallowed exception.
 */
export function copyToClipboard(
  text: string,
  label: string,
  setToast: (msg: string) => void,
): void {
  void navigator.clipboard.writeText(text).then(
    () => setToast(`Copied ${label}.`),
    (e) =>
      setToast(
        `Couldn't copy ${label}: ${e instanceof Error ? e.message : String(e)}`,
      ),
  );
}
