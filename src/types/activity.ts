// Live session deltas + activity card + trends DTOs.
// Sharded from src/types.ts to keep each domain's DTOs in its own
// file; src/types/index.ts re-exports them. Mirrors src-tauri/src/dto.rs.


/** Canonical live-status vocabulary — matches CC's own
 * `concurrentSessions::SessionStatus`. Extra overlays (`errored`,
 * `stuck`) live as separate booleans on `LiveSessionSummary`. */
export type LiveStatus = "busy" | "idle" | "waiting";

/** One row in the live session list. Mirrors
 * `LiveSessionSummaryDto` on the Rust side. Every user-content field
 * (`current_action`) has already passed through the redactor before
 * crossing the IPC boundary — the webview never sees unredacted
 * tokens. */
export interface LiveSessionSummary {
  session_id: string;
  pid: number;
  cwd: string;
  transcript_path: string | null;
  status: LiveStatus;
  current_action: string | null;
  model: string | null;
  waiting_for: string | null;
  errored: boolean;
  stuck: boolean;
  idle_ms: number;
  seq: number;
}

/** Per-session delta kind. Discriminated on `kind` (snake_case on
 * the wire), matching the Rust `LiveDeltaKindDto` serde layout. */
export type LiveDeltaKind =
  | { kind: "status_changed"; status: LiveStatus; waiting_for: string | null }
  | { kind: "task_summary_changed"; summary: string }
  | { kind: "model_changed"; model: string }
  | { kind: "overlay_changed"; errored: boolean; stuck: boolean }
  | { kind: "ended" };

export type LiveDelta = LiveDeltaKind & {
  session_id: string;
  seq: number;
  produced_at_ms: number;
  resync_required: boolean;
};

/** Time-series snapshot for the Activity Trends view. Matches
 *  `ActivityTrendsDto` on the Rust side. `active_series[i]` is the
 *  number of distinct live sessions observed during bucket `i`
 *  (bucket_width_ms wide, starting at `from_ms`). */
export interface ActivityTrends {
  from_ms: number;
  to_ms: number;
  bucket_width_ms: number;
  active_series: number[];
  error_count: number;
}

// ---------------------------------------------------------------------------
// Activity cards — per-event forensic surface
// (separate from ActivityTrends's live-strip aggregation; see design v2)
// ---------------------------------------------------------------------------

export type CardKindLabel =
  | "hook"
  | "hook-slow"
  | "hook-info"
  | "agent"
  | "agent-stranded"
  | "tool-error"
  | "command"
  | "milestone";

export type SeverityLabel = "INFO" | "NOTICE" | "WARN" | "ERROR";

export interface HelpRef {
  template_id: string;
  args: Record<string, string>;
  /** Pre-rendered English text from the template catalog. `undefined`
   *  means the template id was unknown to the binary that wrote it —
   *  the renderer should hide the help line rather than guess. */
  rendered?: string;
}

export interface SourceRef {
  path: string;
  line?: number;
  scope: "project" | "local" | "user" | "managed" | "unknown";
}

/** One activity card. Matches `ActivityCardDto` on the Rust side. */
export interface ActivityCard {
  id: number;
  session_path: string;
  event_uuid?: string;
  byte_offset: number;
  kind: CardKindLabel;
  ts_ms: number;
  severity: SeverityLabel;
  title: string;
  subtitle?: string;
  help?: HelpRef;
  source_ref?: SourceRef;
  cwd: string;
  git_branch?: string;
  plugin?: string;
}

/** Filter set for `cardsRecent` / `cardsCountNewSince`. Every field
 *  optional; absent = no constraint on that dimension. */
export interface CardsRecentQuery {
  sinceMs?: number;
  kinds?: CardKindLabel[];
  minSeverity?: "info" | "notice" | "warn" | "error";
  projectPathPrefix?: string;
  plugin?: string;
  limit?: number;
}

export interface CardsCount {
  total: number;
  /** Cards with id strictly above `lastSeenId`. The "N new since you
   *  were away" badge value. */
  new: number;
  lastSeenId?: number | null;
}

/** Click-through navigation payload. The renderer uses this to
 *  switch to the Sessions section and seek to the right line. */
export interface CardNavigate {
  sessionPath: string;
  byteOffset: number;
  eventUuid?: string;
}

export interface CardsReindexFailure {
  path: string;
  error: string;
}

export interface CardsReindexResult {
  filesScanned: number;
  cardsInserted: number;
  cardsSkippedDuplicates: number;
  cardsPruned: number;
  failed: CardsReindexFailure[];
  elapsedMs: number;
}
