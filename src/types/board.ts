// Board DTOs. Mirrors `src-tauri/src/commands/board.rs` and
// `claudepot_core::board::widget`.
//
// # Provenance is a claim, not a fact
//
// There is no authenticated write channel (plan §11), so a writer's
// identity is self-reported. Every field carrying one is named
// `reported_*` on purpose — a renderer that wants to present it as
// verified has to rename it first, which is exactly the friction the
// naming exists to create. See plan §8.5.

/** A label already truncated for display, with the full text kept. */
export interface BoardLabel {
  text: string;
  /** Present only when `text` was shortened. Feed it to `title`. */
  full?: string;
}

/**
 * Internally tagged, matching `#[serde(tag = "kind")]` on the Rust
 * `AxisScale`.
 *
 * The first cut modelled serde's DEFAULT representation, which emits a
 * bare string `"linear"` for unit variants and an object for the struct
 * variant. That made `"log_fell_back_to_linear" in scale` throw
 * `TypeError` on every ordinary chart — and the unit test passed
 * because its fixture encoded the same wrong shape. Rust is now tagged
 * so one discriminant narrows all three.
 */
export type AxisScale =
  | { kind: "linear" }
  | { kind: "log" }
  | { kind: "log_fell_back_to_linear"; reason: string };

export interface AxisRange {
  min: number;
  max: number;
  scale: AxisScale;
  /** True when the data was flat and the range was padded to be drawable. */
  padded: boolean;
}

export interface BoardPoint {
  x: string;
  /** `null` is a GAP. Break the line — never draw zero, never interpolate. */
  y: number | null;
}

/**
 * What to draw. Every degenerate case is already resolved in Rust —
 * the renderer draws this and makes no decisions of its own.
 */
export type RenderPlan =
  | { kind: "empty"; reason: string }
  | {
      kind: "line";
      points: BoardPoint[];
      y_axis: AxisRange;
      /** Non-null when points were dropped. Never render silently truncated. */
      downsampled_from: number | null;
    }
  | {
      kind: "bar";
      points: BoardPoint[];
      y_axis: AxisRange;
      collapsed_categories: number | null;
    }
  | { kind: "kpi"; value: number | null; sample_size: number }
  | {
      kind: "table";
      headers: BoardLabel[];
      rows: string[][];
      total_rows: number;
    };

export interface ResolvedWidget {
  id: string;
  title: BoardLabel;
  plan: RenderPlan;
}

export interface BoardColumn {
  name: string;
  ty: string;
}

export interface BoardSeriesSummary {
  name: string;
  columns: BoardColumn[];
  row_count: number;
  reported_writer: string | null;
  last_pushed_at: string | null;
}

export interface BoardSummary {
  board_id: string;
  name: string;
  spec_revision: number;
  created_at: string;
  updated_at: string;
  series: string[];
  total_rows: number;
  reported_writer: string | null;
}

export interface BoardDetail {
  board_id: string;
  name: string;
  spec_revision: number;
  created_at: string;
  updated_at: string;
  source_board_id: string | null;
  series: BoardSeriesSummary[];
  widgets: ResolvedWidget[];
  /** Rendered verbatim so the caveat cannot drift from the backend. */
  provenance_note: string;
}
