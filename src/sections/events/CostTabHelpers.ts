// Shared helpers for the Cost-tab cluster (CostTab + TopPromptsPanel
// + future drill-down surfaces). Pure functions only — anything
// that needs React state lives in the component file.

/**
 * Prompt-cache hit rate for a row of input-side token totals.
 *
 * Definition matches Anthropic's billing model: every prompt token is
 * categorised as fresh `input`, `cache_creation` (writing the prefix
 * for future hits), or `cache_read` (served from cache). Hit rate is
 * the read share of that pie.
 *
 * Returns `null` when the denominator is zero (no input-side tokens
 * yet — usually a never-active session). Caller renders `—`.
 */
export function cacheHitRate(row: {
  tokens_input: number;
  tokens_cache_creation: number;
  tokens_cache_read: number;
}): number | null {
  const denom =
    row.tokens_input + row.tokens_cache_creation + row.tokens_cache_read;
  if (denom === 0) return null;
  return row.tokens_cache_read / denom;
}

/** Pretty-print a hit rate as `"83%"`, or `"—"` for null. */
export function formatHitRate(r: number | null): string {
  if (r == null) return "—";
  return `${Math.round(r * 100)}%`;
}

/**
 * Strip the leading `claude-` family prefix when rendering a model
 * id in tight column space. `claude-opus-4-7` → `opus-4-7`. Falls
 * back to the raw id for unknown shapes.
 */
export function shortModelId(id: string): string {
  if (id.startsWith("claude-")) return id.slice("claude-".length);
  return id;
}
