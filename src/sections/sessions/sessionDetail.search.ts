import type { SessionChunk, SessionEvent, SessionRow } from "../../types";

/**
 * Search predicates for the session-detail viewer's filter input.
 * Pure functions, no React. Lifted out of `SessionDetail.tsx` so the
 * orchestrator stays focused on layout / data / lifecycle and these
 * predicates are unit-testable in isolation.
 *
 * Query is expected pre-lowercased — the caller does that once per
 * keystroke, not per event/chunk.
 *
 * Defensive coercion: every field this module reads crosses the Tauri
 * boundary. A newer web bundle can meet an older native binary
 * (missing `input_full`, `models: null`), or CC can emit an event with
 * a `null` branch. `safeLower` absorbs those shapes so a hot-path
 * predicate never throws mid-`useMemo`.
 */

/** Coerce any value to a safe lowercase string. See module doc. */
function safeLower(v: unknown): string {
  if (v == null) return "";
  return String(v).toLowerCase();
}

/**
 * Minimum query length. Anything shorter returns "too short" from
 * `normalizeDetailQuery` so the filter-everything branch kicks in.
 * Mirrors the 2-char floor used by `useSessionSearch` and
 * `SearchQuery::new` on the Rust side — consistent across layers.
 */
export const DETAIL_QUERY_MIN_LEN = 2;

/**
 * Turn the raw input value into the canonical form every detail-level
 * predicate expects: trimmed + lowercased, or `null` when the trimmed
 * form is shorter than `DETAIL_QUERY_MIN_LEN`.
 *
 * Pulling this out of `SessionDetail.tsx` removes the long-standing
 * trap where one code path trimmed before lowercasing and another
 * didn't — a `" tauri "` query used to filter the transcript on `
 * tauri ` (no match) while classifying meta fields against `tauri`
 * (match). Callers now share one derivation.
 */
export function normalizeDetailQuery(raw: string): string | null {
  const q = raw.trim().toLowerCase();
  return q.length >= DETAIL_QUERY_MIN_LEN ? q : null;
}

/**
 * Tauri emits command-not-found errors in a small set of shapes. We
 * match only those exact shapes — specifically, a "not found" wording
 * scoped to a command context — so genuine I/O errors that happen to
 * contain the substring "not found" (e.g. "file not found at <path>")
 * do NOT get classified as a compatibility miss. Unknown real errors
 * must surface to the user, not silently collapse the debugger into
 * raw-event mode.
 */
export function isUnknownCommandError(e: unknown): boolean {
  const msg = String(e ?? "");
  return (
    /unknown command\b/i.test(msg) ||
    /\bcommand\s+[`"']?session_chunks[`"']?\s+not\s+found/i.test(msg) ||
    /not a registered command/i.test(msg)
  );
}

/**
 * True if any event under this chunk matches `q`. `events` is the
 * full event list and `chunk` carries indices into it.
 *
 * An earlier revision also looped over `chunk.tool_executions` to
 * match `tool_name` / `input_full` / `result_content`. Those fields
 * are already reachable through the per-event scan: the paired
 * `AssistantToolUse` and `UserToolResult` events live in
 * `event_indices` too, so the second pass was duplicate work on the
 * largest payloads in the chunk — one scan per keystroke is enough.
 */
export function chunkMatchesSearch(
  chunk: SessionChunk,
  events: SessionEvent[],
  q: string,
): boolean {
  const indices =
    chunk.chunkType === "ai" ? chunk.event_indices : [chunk.event_index];
  for (const idx of indices) {
    const ev = events[idx];
    if (ev && eventMatchesSearch(ev, q)) return true;
  }
  return false;
}

/**
 * A single row-level meta match: which field carried the query, and
 * the value that matched. Used by the detail viewer to explain *why*
 * a session surfaced in the list filter when the transcript itself
 * contains no hits.
 *
 * `field` is a user-facing label (`"project path"`, `"branch"`, …) —
 * not a programmatic key. The UI renders it directly.
 */
export interface MetaMatch {
  field: string;
  value: string;
}

/**
 * Report which row-level metadata fields — the ones the list filter
 * scans but the transcript doesn't carry — contain `qLower`. Empty
 * result means the list hit came from transcript content only, and
 * the regular "Nothing matches that query" message is appropriate.
 *
 * `qLower` must already be lowercased and length-checked by the
 * caller; this helper does no normalization so it stays referentially
 * transparent inside a `useMemo`.
 */
export function classifyMetaMatch(
  row: SessionRow,
  qLower: string,
): MetaMatch[] {
  if (qLower.length === 0) return [];
  const out: MetaMatch[] = [];
  const projectPath = row.project_path ?? "";
  if (safeLower(projectPath).includes(qLower)) {
    out.push({ field: "project path", value: projectPath });
  }
  const branch = row.git_branch ?? "";
  if (branch && safeLower(branch).includes(qLower)) {
    out.push({ field: "branch", value: branch });
  }
  // `models` is the historical list of every model that touched the
  // session; report the first match so the UI stays compact. An
  // older Tauri binary could deliver `null` instead of `[]` — coerce.
  const models = Array.isArray(row.models) ? row.models : [];
  const modelHit = models.find((m) => safeLower(m).includes(qLower));
  if (modelHit) {
    out.push({ field: "model", value: modelHit });
  }
  // Session id prefix match mirrors the list-level fast path — same
  // field, same predicate.
  const sessionId = row.session_id ?? "";
  if (safeLower(sessionId).startsWith(qLower)) {
    out.push({ field: "session id", value: sessionId });
  }
  return out;
}

/**
 * True if any text-bearing field on the event matches `q`. Variants
 * with no user-controlled text (`fileSnapshot`) are deliberately
 * excluded — searching them would never match anyway.
 */
export function eventMatchesSearch(e: SessionEvent, q: string): boolean {
  switch (e.kind) {
    case "userText":
    case "assistantText":
    case "assistantThinking":
    case "summary":
      return safeLower(e.text).includes(q);
    case "userToolResult":
      return (
        safeLower(e.content).includes(q) || safeLower(e.tool_use_id).includes(q)
      );
    case "assistantToolUse":
      // Prefer `input_full` (untruncated JSON, matches what the
      // list-level deep search scans). Fall back to `input_preview`
      // when the field is missing — that path only matters if a
      // newer web bundle talks to an older Tauri binary that hasn't
      // shipped `input_full` yet.
      return (
        safeLower(e.tool_name).includes(q) ||
        safeLower(e.input_full || e.input_preview).includes(q)
      );
    case "system":
      return safeLower(e.subtype).includes(q) || safeLower(e.detail).includes(q);
    case "attachment":
      return safeLower(e.name).includes(q);
    case "fileSnapshot":
      return false;
    case "other":
      return safeLower(e.raw_type).includes(q);
    case "malformed":
      return safeLower(e.preview).includes(q);
  }
}
