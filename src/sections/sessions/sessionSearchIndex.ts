import type { SessionRow } from "../../types";

/**
 * Pre-lowercased haystack for the in-memory session filter. Built once
 * per `sessions` array (via `useMemo` in the caller) and reused across
 * every keystroke, so `.toLowerCase()` runs N times per mount rather
 * than N×K times per keystroke.
 *
 * We keep a parallel `Map<file_path, string>` instead of mutating the
 * row objects — rows cross the Tauri boundary as frozen-by-convention
 * DTOs, and inline caches on them would leak into the detail panel
 * and break React reconciliation keys.
 */
export interface SessionSearchHaystack {
  get(filePath: string): string | undefined;
}

export function buildSessionSearchHaystack(
  sessions: readonly SessionRow[],
): SessionSearchHaystack {
  const map = new Map<string, string>();
  for (const s of sessions) {
    const parts: string[] = [
      s.session_id,
      s.project_path,
      s.first_user_prompt ?? "",
      s.git_branch ?? "",
    ];
    for (const m of s.models) parts.push(m);
    map.set(s.file_path, parts.join("").toLowerCase());
  }
  return {
    get: (filePath) => map.get(filePath),
  };
}

/**
 * Pure filter. `sessionIdPrefix` runs first because the CC id space is
 * dense and a matched prefix is an exact signal. The haystack match is
 * a substring over the concatenated lowercased fields.
 */
export function matchesQuery(
  session: SessionRow,
  haystack: SessionSearchHaystack,
  qLower: string,
): boolean {
  if (qLower.length === 0) return true;
  if (session.session_id.toLowerCase().startsWith(qLower)) return true;
  const hay = haystack.get(session.file_path);
  if (hay !== undefined && hay.includes(qLower)) return true;
  return false;
}
