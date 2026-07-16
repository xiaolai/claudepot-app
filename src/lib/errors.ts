// Turn a Tauri command rejection into a message fit for a user.
//
// Every shared-memory command returns `Result<T, String>`, so a rejected
// `invoke` resolves to a plain string — but one written for a developer:
// prefixed with the internal command label (`read: …`, `counts_by_project:
// …`) and carrying raw IO/thiserror text. IPC-level failures (command not
// registered, arg deserialization) can instead reject with an `Error`
// object, which `String(e)` would render as `[object Object]`. Route every
// catch through here so the surface shows something honest and readable.

/** Pull a readable string out of any thrown/rejected value, never
 *  surfacing the useless `"[object Object]"`. */
function extractMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  if (
    e != null &&
    typeof e === "object" &&
    "message" in e &&
    typeof (e as { message: unknown }).message === "string"
  ) {
    return (e as { message: string }).message;
  }
  if (e == null) return "";
  const s = String(e);
  // A plain object stringifies to "[object Object]" — the exact thing this
  // module exists to avoid. Treat it as "no message".
  return s === "[object Object]" ? "" : s;
}

/** A readable, non-developer message for any thrown/rejected value. */
export function toUserError(e: unknown): string {
  const raw = extractMessage(e);

  // Known backend failure classes → plain guidance the user can act on.
  if (/session index unavailable/i.test(raw))
    return "The knowledge index couldn't be opened. Restart Claudepot, or rebuild it from Settings → Cleanup.";
  if (/blocking task (failed|panicked)/i.test(raw))
    return "That operation failed unexpectedly. Try again; if it keeps happening, restart Claudepot.";

  // Strip a leading internal command label ("read: …", "counts_by_project:
  // …") — it means nothing to a user — but keep the substance after it. The
  // strip is deliberately case-SENSITIVE and single-token: backend labels
  // are lowercase snake_case, so this won't eat a capitalized sentence lead
  // like "Error: disk full" or "HTTP: 500".
  const stripped = raw.replace(/^[a-z_]+:\s+/, "").trim();
  return stripped.length > 0 ? stripped : "Something went wrong.";
}

/** A read_locator failure is almost always a moved/pruned transcript.
 *  Callers that read an excerpt use this for a message the user understands. */
export function toExcerptError(e: unknown): string {
  const raw = extractMessage(e);
  // `\bmoved\b` so "file removed by user" isn't misread as "moved".
  if (/no such file|not found|os error 2|\bmoved\b/i.test(raw))
    return "This exchange is no longer available — the transcript may have moved or been pruned.";
  return toUserError(e);
}
