/**
 * When did a session's work actually happen?
 *
 * Two timestamps travel on every `SessionRow` and they answer different
 * questions:
 *
 * - `last_ts` — the last event timestamp parsed out of the transcript.
 *   This is when the tokens were actually spent.
 * - `last_modified_ms` — the `.jsonl` file's mtime. This is when the
 *   file was last *written*, which a move, a re-index, a backup restore,
 *   or a `slim` rewrite all change without any work happening.
 *
 * For display ("3 days ago") the difference rarely matters. For
 * **dated pricing** it does: scoring a session at the rate in force on
 * its mtime re-prices old work every time the file is touched, which is
 * exactly the silent-rewrite problem the dated rate book exists to
 * prevent. `claudepot_core::usage_local` prices from `last_ts`; this
 * helper is how the renderer stays consistent with it.
 *
 * Prefer the event timestamp, fall back to mtime only when the
 * transcript carries no usable one (an empty or unparsed session).
 */
export function sessionEventMs(
  lastTs: string | null,
  lastModifiedMs: number | null,
): number | null {
  if (lastTs) {
    const t = Date.parse(lastTs);
    if (!Number.isNaN(t)) return t;
  }
  return lastModifiedMs;
}
