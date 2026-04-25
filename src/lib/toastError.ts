import { redactSecrets } from "./redactSecrets";

/**
 * Hard cap for toast text. Toasts live in a small floating row at
 * the bottom of the window; a 1 KB stack trace would push the
 * primary action off-screen and the user can't read past the first
 * line anyway. 240 chars is enough for "Sync failed: 401 Unauthorized
 * (refresh_token expired — sign in again)" plus a little headroom.
 */
const MAX_TOAST_LEN = 240;

/**
 * Format an unknown thrown value into a user-visible toast message,
 * redacting any `sk-ant-*` token and bounding the length.
 *
 * The CLAUDE.md `rules/rust-conventions.md` token-redaction rule
 * applies just as much to JS toasts as to log lines — a `pushToast(
 * "error", \`Sync failed: ${e}\`)` happily renders an oauth blob if
 * the backend echoes one back in the error body, and that blob then
 * sits in the DOM until the toast auto-dismisses (10 s) or the user
 * screenshots it.
 *
 * Format: `<scope>: <redacted, truncated message>`. `scope` is a
 * short verb like "Sync" or "Verify all" so the user knows which
 * action failed.
 */
export function formatErrorMessage(scope: string, e: unknown): string {
  const raw = e instanceof Error ? e.message : String(e);
  const redacted = redactSecrets(raw);
  const trimmed =
    redacted.length <= MAX_TOAST_LEN
      ? redacted
      : `${redacted.slice(0, MAX_TOAST_LEN - 1)}…`;
  return `${scope}: ${trimmed}`;
}

/**
 * Convenience wrapper that pushes a redacted error toast through the
 * caller-supplied `pushToast` setter. Pulling this into a helper means
 * App.tsx and any other ad-hoc handler share one redaction pipeline,
 * so a future change to the rules (e.g. masking new token shapes)
 * lands in exactly one place.
 */
export function toastError(
  pushToast: (kind: "info" | "error", text: string) => void,
  scope: string,
  e: unknown,
): void {
  pushToast("error", formatErrorMessage(scope, e));
}
