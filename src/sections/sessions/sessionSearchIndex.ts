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
 *
 * Fields are joined with a `\n` separator to prevent substring matches
 * that straddle two fields (e.g. the tail of `project_path` merging
 * with the head of `first_user_prompt` into a spurious hit). `\n` is
 * cheap and will never appear inside a CC-emitted cwd, branch name, or
 * model id.
 */
export interface SessionSearchHaystack {
  get(filePath: string): string | undefined;
}

/** Field separator for the concatenated haystack. See module doc. */
const FIELD_SEP = "\n";

/**
 * Coerce any value to a safe lowercase string. Absorbs null/undefined
 * that could sneak across the Tauri boundary despite the TypeScript
 * contract, rather than throwing inside a hot-path `useMemo`. Zero
 * trust at boundaries is cheaper than a crashed section.
 */
function safeLower(v: unknown): string {
  if (v == null) return "";
  return String(v).toLowerCase();
}

export function buildSessionSearchHaystack(
  sessions: readonly SessionRow[],
): SessionSearchHaystack {
  const map = new Map<string, string>();
  for (const s of sessions) {
    const parts: string[] = [
      safeLower(s.session_id),
      safeLower(s.project_path),
      safeLower(s.first_user_prompt),
      safeLower(s.git_branch),
    ];
    // `s.models` is `string[]` at the type level but the Tauri DTO
    // could drop to null on an older binary — a null iterable would
    // throw and crash the memo. Coerce defensively.
    const models = Array.isArray(s.models) ? s.models : [];
    for (const m of models) parts.push(safeLower(m));
    map.set(s.file_path, parts.join(FIELD_SEP));
  }
  return {
    get: (filePath) => map.get(filePath),
  };
}

/**
 * Pure filter. `sessionIdPrefix` runs first because the CC id space is
 * dense and a matched prefix is an exact signal. The haystack match is
 * a substring over the separator-joined lowercased fields.
 */
export function matchesQuery(
  session: SessionRow,
  haystack: SessionSearchHaystack,
  qLower: string,
): boolean {
  if (qLower.length === 0) return true;
  if (safeLower(session.session_id).startsWith(qLower)) return true;
  const hay = haystack.get(session.file_path);
  if (hay !== undefined && hay.includes(qLower)) return true;
  return false;
}
