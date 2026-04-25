import type { ReactNode } from "react";
import type { RunningOpInfo } from "../../types";
import type { PhaseSpec } from "../projects/OperationProgressModal";

/**
 * Phase ids + labels emitted by the login progress sink. Stable contract
 * with `claudepot_core::services::account_service::LoginPhase` (the Tauri
 * adapter in `src-tauri/src/ops.rs::TauriLoginProgressSink` writes the
 * snake_case names matching `LoginPhase::as_str`). The labels are kept
 * short so the row reads well at the modal's default width.
 */
export const LOGIN_PHASES: PhaseSpec[] = [
  { id: "spawning", label: "Preparing" },
  { id: "waiting_for_browser", label: "Waiting for browser" },
  { id: "reading_blob", label: "Reading credentials" },
  { id: "fetching_profile", label: "Fetching profile" },
  { id: "verifying_identity", label: "Verifying identity" },
  { id: "persisting", label: "Saving" },
];

/**
 * Render the success-state body for a login op. The terminal `RunningOpInfo`
 * doesn't carry a structured login result (the side effects — credentials
 * persisted, verify_status set — show up in the next account list refresh),
 * so the success body is intentionally sparse.
 */
export function renderLoginResult(_info: RunningOpInfo | null): ReactNode {
  return null;
}
