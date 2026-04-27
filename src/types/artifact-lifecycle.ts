// Mirrors `src-tauri/src/dto_artifact_lifecycle.rs`. All path fields
// arrive as strings (display form). The triple
// `(scope_root, kind, relative_path)` is canonical addressing — the
// renderer should always pass these together rather than abs paths.

export type LifecycleScope = "user" | "project";
export type LifecycleKind = "skill" | "agent" | "command";
export type PayloadKind = "file" | "directory";
export type OnConflict = "refuse" | "suffix";

/**
 * Trash-entry health state. `Healthy` is the only state that
 * supports one-click `restore`. `MissingManifest` and
 * `AbandonedStaging` go through `recover` (with a confirmed target).
 * `MissingPayload` and `OrphanPayload` only support `forget`.
 */
export type TrashState =
  | "healthy"
  | "missing_manifest"
  | "missing_payload"
  | "orphan_payload"
  | "abandoned_staging";

export interface DisabledRecordDto {
  scope: LifecycleScope;
  scope_root: string;
  kind: LifecycleKind;
  /** Slash-joined path under <kind> dir. */
  name: string;
  /** Active path the artifact would land at on enable / restore. */
  original_path: string;
  /** Current on-disk location (under .disabled/<kind>/...). */
  current_path: string;
  payload_kind: PayloadKind;
}

export interface TrashManifestDto {
  scope: LifecycleScope;
  scope_root: string;
  kind: LifecycleKind;
  relative_path: string;
  original_path: string;
  source_basename: string;
  payload_kind: PayloadKind;
  byte_count: number;
  sha256: string | null;
}

export interface TrashEntryDto {
  id: string;
  entry_dir: string;
  state: TrashState;
  trashed_at_ms: number | null;
  manifest: TrashManifestDto | null;
}

export interface RestoredArtifactDto {
  id: string;
  final_path: string;
}

export interface TrackableDto {
  scope: LifecycleScope;
  scope_root: string;
  kind: LifecycleKind;
  relative_path: string;
  payload_kind: PayloadKind;
}

export interface ClassifyPathDto {
  /** Present when the path is eligible for a lifecycle action. */
  trackable: TrackableDto | null;
  /** Human-readable refusal reason from RefuseReason::Display. */
  refused: string | null;
  /** True when the trackable points into a `.disabled/...` location. */
  already_disabled: boolean;
}
