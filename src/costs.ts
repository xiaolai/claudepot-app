import { useEffect, useState } from "react";
import { api } from "./api";
import { i18n } from "./lib/i18n";
import type {
  ModelRatesDto,
  PremiumUsage,
  PriceBookSnapshotDto,
  PriceTableDto,
  RatePeriodDto,
  TokenCounts,
} from "./types";

/**
 * Token usage sufficient for cost estimation — the session row's
 * `tokens` field. Absent fields count as zero: older transcripts
 * predate prompt caching, and older rows predate one-hour writes.
 */
export type TokenUsage = TokenCounts;

/** A calendar day as `[year, month, day]`, matching the Rust `Ymd`. */
export type Ymd = [number, number, number];

/** How well a resolved rate matches the model it was asked about. */
export type RateConfidence = "exact" | "family_estimate";

export interface ResolvedRates {
  rates: ModelRatesDto;
  confidence: RateConfidence;
}

/** A cost figure plus how much to trust the rate behind it. */
export interface PricedCost {
  usd: number;
  confidence: RateConfidence;
}

/**
 * Canonicalize a CC-reported model id. Mirrors
 * `session_live::pricing::canonicalize_model_id`: lowercase, drop a
 * trailing `-YYYYMMDD` snapshot stamp, drop an alias marker.
 */
export function canonicalizeModelId(raw: string): string {
  return raw
    .toLowerCase()
    .replace(/-\d{8}$/, "")
    .replace(/-(preview|latest|experimental)$/, "");
}

/**
 * `claude-opus-5` → `claude-opus-`. `null` for anything not shaped
 * like `claude-<family>-…`.
 */
function familyPrefix(id: string): string | null {
  const CLAUDE = "claude-";
  if (!id.startsWith(CLAUDE)) return null;
  const rest = id.slice(CLAUDE.length);
  const dash = rest.indexOf("-");
  if (dash < 0) return null;
  return id.slice(0, CLAUDE.length + dash + 1);
}

/** Chronological compare of two `Ymd`s. */
function ymdLte(a: Ymd, b: Ymd): boolean {
  if (a[0] !== b[0]) return a[0] < b[0];
  if (a[1] !== b[1]) return a[1] < b[1];
  return a[2] <= b[2];
}

/**
 * The last period that had begun on or before `on`. Mirrors
 * `session_live::pricing::rate_on`; requires periods oldest-first,
 * which the backend snapshot guarantees.
 */
function rateOn(
  periods: readonly RatePeriodDto[],
  on: Ymd,
): ModelRatesDto | null {
  for (let i = periods.length - 1; i >= 0; i--) {
    const p = periods[i];
    if (p.starts === null || ymdLte(p.starts as Ymd, on)) {
      return ratesOf(p);
    }
  }
  return null;
}

function ratesOf(p: RatePeriodDto): ModelRatesDto {
  return {
    input_per_mtok: p.input_per_mtok,
    output_per_mtok: p.output_per_mtok,
    cache_write_per_mtok: p.cache_write_per_mtok,
    cache_read_per_mtok: p.cache_read_per_mtok,
    cache_write_1h_per_mtok: p.cache_write_1h_per_mtok,
  };
}

/** Today in UTC, matching the backend's `today_utc`. */
export function todayUtc(): Ymd {
  const d = new Date();
  return [d.getUTCFullYear(), d.getUTCMonth() + 1, d.getUTCDate()];
}

/** Epoch milliseconds → UTC calendar day. */
export function ymdFromMs(ms: number): Ymd | null {
  if (!Number.isFinite(ms)) return null;
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return null;
  return [d.getUTCFullYear(), d.getUTCMonth() + 1, d.getUTCDate()];
}

/**
 * Resolve a model id against the dated rate book as it stood on `on`.
 *
 * This mirrors `claudepot_core::pricing::PriceBook::resolve`, and the
 * two are locked together by
 * `crates/claudepot-core/testdata/rate-resolution-vectors.json` — both
 * run those vectors. **Change one, change the other.**
 *
 * 1. Exact — the canonicalized id is in the book.
 * 2. Family estimate — the id isn't listed but its family is, so
 *    borrow the family's current model's rate for that same day. The
 *    returned `confidence` says so; the UI must mark it.
 * 3. `null` — no family match, rendered `—` rather than `$0.00`.
 */
export function resolveRatesOn(
  book: PriceBookSnapshotDto | null | undefined,
  modelId: string,
  on: Ymd,
): ResolvedRates | null {
  if (!book) return null;
  const key = canonicalizeModelId(modelId.trim());

  const exact = book.models[key];
  if (exact) {
    const rates = rateOn(exact, on);
    if (rates) return { rates, confidence: "exact" };
  }

  const family = familyPrefix(key);
  if (!family) return null;
  const current = book.family_current[family];
  if (!current) return null;
  const periods = book.models[current];
  if (!periods) return null;
  const rates = rateOn(periods, on);
  return rates ? { rates, confidence: "family_estimate" } : null;
}

/**
 * Token cost of `u` at `r`. Mirrors `session_live::pricing::price_tokens`:
 * one-hour writes at their own rate, capped at the write total, the
 * rest of the writes at the five-minute rate.
 */
function priceTokens(r: ModelRatesDto, u: TokenCounts): number {
  const n = (v: number | undefined) => v ?? 0;
  const write = n(u.cache_creation);
  const write1h = Math.min(n(u.cache_creation_1h), write);
  return (
    (n(u.input) * r.input_per_mtok +
      n(u.output) * r.output_per_mtok +
      n(u.cache_read) * r.cache_read_per_mtok +
      (write - write1h) * r.cache_write_per_mtok +
      write1h * r.cache_write_1h_per_mtok) /
    1_000_000
  );
}

const TOKEN_FIELDS = [
  "input",
  "output",
  "cache_creation",
  "cache_read",
  "cache_creation_1h",
  "web_search_requests",
] as const;

/** Field-wise `a - b`, floored at zero. */
function minus(a: TokenCounts, b: TokenCounts | undefined): TokenCounts {
  if (!b) return a;
  const out: TokenCounts = {};
  for (const k of TOKEN_FIELDS) out[k] = Math.max(0, (a[k] ?? 0) - (b[k] ?? 0));
  return out;
}

/**
 * Compute hypothetical API cost for the given usage at the rate in
 * force on `on`. Returns `null` when the model belongs to no priced
 * family — the UI should render "rate unknown" rather than $0.00.
 *
 * Mirrors `claudepot_core::pricing::PriceBook::cost`, locked together
 * by `crates/claudepot-core/testdata/cost-vectors.json`: fast-mode
 * tokens at the model's fast rate when it has one, US-only tokens at
 * 1.1×, web searches per request and never multiplied. `premium` holds
 * subsets of `usage`; the standard remainder floors at zero.
 *
 * `on` defaults to today, which is correct for live sessions. Anything
 * scoring historical usage must pass that usage's own day, or a
 * session from before a price change is re-scored at today's rate.
 */
export function costFromUsage(
  table: PriceTableDto | null,
  modelId: string,
  usage: TokenUsage,
  on: Ymd = todayUtc(),
  premium?: PremiumUsage,
): PricedCost | null {
  const book = table?.book;
  const resolved = resolveRatesOn(book, modelId, on);
  if (!book || !resolved) return null;
  const base = resolved.rates;
  const fastPeriod = book.fast_models[canonicalizeModelId(modelId.trim())];
  const fast = fastPeriod ? ratesOf(fastPeriod) : base;
  const standard = minus(
    minus(minus(usage, premium?.fast), premium?.us),
    premium?.fast_us,
  );
  const geo = book.us_geo_multiplier;
  const tokensUsd =
    priceTokens(base, standard) +
    priceTokens(base, premium?.us ?? {}) * geo +
    priceTokens(fast, premium?.fast ?? {}) +
    priceTokens(fast, premium?.fast_us ?? {}) * geo;
  const searchesUsd =
    (usage.web_search_requests ?? 0) * book.web_search_usd_per_request;
  return { usd: tokensUsd + searchesUsd, confidence: resolved.confidence };
}

/**
 * Session-level cost estimate. If the session bounced between
 * several models, caller passes the dominant one (or the first
 * model in the array). When multiple models were used, the estimate
 * is necessarily approximate — the exact per-message breakdown
 * isn't summed at the session level today; this matches what the
 * dashboard needs for at-a-glance display, not line-item billing.
 *
 * `atMs` is the session's timestamp; pass it so a session that ran
 * before a rate change keeps its original cost.
 */
export function sessionCostEstimate(
  table: PriceTableDto | null,
  models: string[],
  usage: TokenUsage,
  atMs?: number | null,
  premium?: PremiumUsage,
): PricedCost | null {
  // `<synthetic>` is Claude Code's placeholder for a turn it generated
  // locally (an API error, "No response requested."), not a model. It
  // has to go before `models[0]` is read: the list arrives sorted from
  // a Rust `BTreeSet`, and `<` sorts ahead of every lowercase letter,
  // so any session that ever hit one API error led with the
  // placeholder and priced as null. Mirrors
  // `session_live::pricing::is_synthetic_model` — see its docs for
  // the measurement.
  const real = models.filter((m) => m !== SYNTHETIC_MODEL);
  if (real.length === 0) return null;
  // Prefer the first model as the basis. Over-estimates slightly
  // when a session starts on Opus and switches to Haiku mid-way
  // (Haiku is 15× cheaper input); under-estimates in the reverse.
  // Acceptable for a dashboard figure — anyone who cares about a
  // precise bill goes to Anthropic's dashboard, not ours.
  const on = (atMs != null ? ymdFromMs(atMs) : null) ?? todayUtc();
  return costFromUsage(table, real[0], usage, on, premium);
}

/**
 * Claude Code's literal placeholder in a transcript's `model` field.
 * Kept in lock-step with `session_live::pricing::SYNTHETIC_MODEL`.
 */
export const SYNTHETIC_MODEL = "<synthetic>";

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
 * Prefix an estimated figure with `≈`. Colour never carries the
 * distinction on its own (design.md's accessibility floor), so the
 * marker is in the text and the caller pairs it with a `title`.
 */
export function formatCost(cost: PricedCost): string {
  const s = formatUsd(cost.usd);
  return cost.confidence === "family_estimate" ? `≈ ${s}` : s;
}

/**
 * Tooltip copy explaining why a figure is marked estimated.
 *
 * A function, not a `const`: a string evaluated at module load freezes
 * the boot language, so a tooltip rendered after a language switch
 * would keep the old one. Resolved at call time instead.
 */
export function estimatedRateHint(): string {
  return i18n.t("cost.estimatedRateHint");
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
