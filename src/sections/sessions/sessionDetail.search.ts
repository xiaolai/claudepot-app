import type { SessionChunk, SessionEvent } from "../../types";

/**
 * Search predicates for the session-detail viewer's filter input.
 * Pure functions, no React. Lifted out of `SessionDetail.tsx` so the
 * orchestrator stays focused on layout / data / lifecycle and these
 * predicates are unit-testable in isolation.
 *
 * Query is expected pre-lowercased — the caller does that once per
 * keystroke, not per event/chunk.
 */

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
 * True if any event under this chunk (and, for AI chunks, any linked
 * tool execution) matches `q`. `events` is the full event list and
 * `chunk` carries indices into it.
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
  // AI chunks: also match against linked tool calls.
  if (chunk.chunkType === "ai") {
    for (const t of chunk.tool_executions) {
      if (
        t.tool_name.toLowerCase().includes(q) ||
        t.input_preview.toLowerCase().includes(q) ||
        (t.result_content ?? "").toLowerCase().includes(q)
      ) {
        return true;
      }
    }
  }
  return false;
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
      return e.text.toLowerCase().includes(q);
    case "userToolResult":
      return e.content.toLowerCase().includes(q) || e.tool_use_id.includes(q);
    case "assistantToolUse":
      return (
        e.tool_name.toLowerCase().includes(q) ||
        e.input_preview.toLowerCase().includes(q)
      );
    case "system":
      return (
        (e.subtype ?? "").toLowerCase().includes(q) ||
        e.detail.toLowerCase().includes(q)
      );
    case "attachment":
      return (e.name ?? "").toLowerCase().includes(q);
    case "fileSnapshot":
      return false;
    case "other":
      return e.raw_type.toLowerCase().includes(q);
    case "malformed":
      return e.preview.toLowerCase().includes(q);
  }
}
