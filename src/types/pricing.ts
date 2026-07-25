// Per-million-token pricing tables.
// Sharded from src/types.ts to keep each domain's DTOs in its own
// file; src/types/index.ts re-exports them. Mirrors src-tauri/src/dto.rs.


// ---------- Pricing ---------------------------------------------------

/** Per-million-token US-dollar rates for one Claude model. */
export interface ModelRatesDto {
  input_per_mtok: number;
  output_per_mtok: number;
  cache_write_per_mtok: number;
  cache_read_per_mtok: number;
}

/** Where the current price table came from. */
export interface PriceSourceDto {
  /** "bundled" | "live" | "cached" */
  kind: "bundled" | "live" | "cached";
  /** ISO-ish timestamp for live / cached; verification date for bundled. */
  timestamp: string;
  /** Source URL (empty for bundled). */
  url: string;
}

/** One rate period from the dated book. `starts: null` is the opening
 *  period; otherwise `[year, month, day]`, the first day it applied. */
export interface RatePeriodDto {
  starts: [number, number, number] | null;
  input_per_mtok: number;
  output_per_mtok: number;
  cache_write_per_mtok: number;
  cache_read_per_mtok: number;
}

/** The dated rate book: every priced model's periods plus the family
 *  fallback map. Mirrors `claudepot_core::pricing::book::PriceBookSnapshot`. */
export interface PriceBookSnapshotDto {
  /** Model id → periods, oldest first. */
  models: Record<string, RatePeriodDto[]>;
  /** `claude-<family>-` → the model id an unlisted member falls back to. */
  family_current: Record<string, string>;
}

export interface PriceTableDto {
  /** Keyed by canonical model id (e.g. `claude-opus-4-7`). Current
   *  rates only — use `book` for anything date-sensitive. */
  models: Record<string, ModelRatesDto>;
  source: PriceSourceDto;
  /** Short user-safe message when the last refresh attempt failed. */
  last_fetch_error: string | null;
  /** The dated rate book all client-side cost math resolves against. */
  book: PriceBookSnapshotDto;
}
