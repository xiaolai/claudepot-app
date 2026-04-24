import { useEffect, useState } from "react";
import { api } from "./api";
import type { ModelRatesDto, PriceTableDto } from "./types";

/**
 * Token usage sufficient for cost estimation. Same shape as the
 * session-detail row's `tokens` field plus a model id so we can look
 * up the right rate. Absent `cache_*` fields default to 0 — older
 * transcripts predate prompt caching.
 */
export interface TokenUsage {
  input: number;
  output: number;
  cache_read?: number;
  cache_creation?: number;
}

/**
 * Resolve a model id against a price table. Mirrors the backend's
 * `resolve_model_rates` rules:
 *
 *   1. Exact id match.
 *   2. Strip a trailing `-YYYYMMDD` suffix and retry.
 *
 * Returns `null` for unknown models so callers can render "rate
 * unknown" instead of silently substituting another model's rate.
 */
export function resolveRates(
  table: PriceTableDto | null,
  modelId: string,
): ModelRatesDto | null {
  if (!table) return null;
  const hit = table.models[modelId];
  if (hit) return hit;
  const m = modelId.match(/^(.+)-(\d{8})$/);
  if (m) {
    const stem = m[1];
    return table.models[stem] ?? null;
  }
  return null;
}

/**
 * Compute hypothetical API cost for the given usage. Returns `null`
 * when the model id can't be resolved — the UI should render "rate
 * unknown" rather than $0.00.
 */
export function costFromUsage(
  table: PriceTableDto | null,
  modelId: string,
  usage: TokenUsage,
): number | null {
  const rates = resolveRates(table, modelId);
  if (!rates) return null;
  const toMtok = (n: number | undefined) => (n ?? 0) / 1_000_000;
  const input = toMtok(usage.input) * rates.input_per_mtok;
  const output = toMtok(usage.output) * rates.output_per_mtok;
  const cacheRead = toMtok(usage.cache_read) * rates.cache_read_per_mtok;
  const cacheWrite = toMtok(usage.cache_creation) * rates.cache_write_per_mtok;
  return input + output + cacheRead + cacheWrite;
}

/**
 * Session-level cost estimate. If the session bounced between
 * several models, caller passes the dominant one (or the first
 * model in the array). When multiple models were used, the estimate
 * is necessarily approximate — the exact per-message breakdown
 * isn't summed at the session level today; this matches what the
 * dashboard needs for at-a-glance display, not line-item billing.
 */
export function sessionCostEstimate(
  table: PriceTableDto | null,
  models: string[],
  usage: TokenUsage,
): number | null {
  if (models.length === 0) return null;
  // Prefer the first model as the basis. Over-estimates slightly
  // when a session starts on Opus and switches to Haiku mid-way
  // (Haiku is 15× cheaper input); under-estimates in the reverse.
  // Acceptable for a dashboard figure — anyone who cares about a
  // precise bill goes to Anthropic's dashboard, not ours.
  return costFromUsage(table, models[0], usage);
}

/**
 * Format a dollar number with adaptive precision — small values
 * keep cents, larger ones round to whole dollars. Strips trailing
 * zeros so `$3.50` doesn't read as `$3.50000`.
 */
export function formatUsd(amount: number): string {
  if (amount >= 100) return `$${amount.toFixed(0)}`;
  if (amount >= 10) return `$${amount.toFixed(1)}`;
  if (amount >= 0.01) return `$${amount.toFixed(2)}`;
  // Sub-penny — show four decimals so users aren't confused by $0.00.
  return `$${amount.toFixed(4)}`;
}

/**
 * React hook: loads the price table once per mount and caches it.
 * Re-exports the promise state so consumers can show a "loading"
 * affordance if they care. Failures leave `table = null` — callers
 * should treat that as "show no cost" rather than "$0.00".
 *
 * The backend's `pricing_get` never blocks and always returns
 * something usable (bundled defaults at worst), so in practice this
 * resolves within a few ms of mount. We don't poll — a day-scale
 * refresh happens server-side on its own cadence; consumers that
 * want "freshness right now" should call `api.pricingRefresh()`.
 */
export function usePriceTable(): {
  table: PriceTableDto | null;
  loading: boolean;
} {
  const [table, setTable] = useState<PriceTableDto | null>(null);
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    let cancelled = false;
    void api
      .pricingGet()
      .then((t) => {
        if (cancelled) return;
        setTable(t);
      })
      .catch(() => {
        if (cancelled) return;
        setTable(null);
      })
      .finally(() => {
        if (cancelled) return;
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);
  return { table, loading };
}
