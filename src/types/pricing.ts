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

export interface PriceTableDto {
  /** Keyed by canonical model id (e.g. `claude-opus-4-7`). */
  models: Record<string, ModelRatesDto>;
  source: PriceSourceDto;
  /** Short user-safe message when the last refresh attempt failed. */
  last_fetch_error: string | null;
}
