/**
 * Formatting helpers shared by the Sessions tab. All numbers are
 * rendered "render-if-nonzero" per design rules — callers should
 * branch on `formatTokens(n)` being empty / null rather than showing
 * "0 tokens" in the UI.
 */

export function formatTokens(n: number): string {
  if (n <= 0) return "";
  if (n < 1_000) return `${n}`;
  if (n < 1_000_000) return `${(n / 1_000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(n < 10_000_000 ? 1 : 0)}M`;
}

/**
 * Humanize the full set of models a session used. Collapses the
 * `claude-<family>-<N>-<variant>` prefix (we keep the family and the
 * dotted version). Keeps the UI calm when a session bounced between
 * Opus 4.7 and Haiku 4.5.
 */
export function modelBadge(models: string[]): string {
  if (models.length === 0) return "";
  const compact = models.map(compactModel);
  if (compact.length === 1) return compact[0];
  const unique = Array.from(new Set(compact));
  if (unique.length === 1) return unique[0];
  return unique.join(" · ");
}

function compactModel(id: string): string {
  const trimmed = id.replace(/^claude-/, "").replace(/-\d{8,}$/, "");
  const parts = trimmed.split("-");
  if (parts.length >= 3) {
    // opus-4-7 → opus 4.7
    const [family, major, minor] = parts;
    return `${family} ${major}.${minor}`;
  }
  return trimmed;
}

/**
 * Short project basename for table rows. `/a/b/c` → `c`.
 * Empty string survives as-is so the caller can fall back to the slug.
 */
export function projectBasename(path: string): string {
  return path.split("/").filter(Boolean).pop() ?? path;
}

export function shortSessionId(id: string): string {
  return id.length >= 8 ? id.slice(0, 8) : id;
}

/**
 * Prefer the live-event `last_ts` timestamp; fall back to file mtime
 * when CC hasn't written a dated event (new empty session).
 */
export function bestTimestampMs(
  lastTs: string | null,
  lastModifiedMs: number | null,
): number | null {
  if (lastTs) {
    const t = Date.parse(lastTs);
    if (!Number.isNaN(t)) return t;
  }
  return lastModifiedMs;
}
