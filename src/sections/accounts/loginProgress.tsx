import type { ReactNode } from "react";
import { i18n } from "../../lib/i18n";
import type { RunningOpInfo } from "../../types";
import type { PhaseSpec } from "../projects/OperationProgressModal";

/**
 * Phase ids + labels emitted by the login progress sink. Stable contract
 * with `claudepot_core::services::account_service::LoginPhase` (the Tauri
 * adapter in `src-tauri/src/ops.rs::TauriLoginProgressSink` writes the
 * snake_case names matching `LoginPhase::as_str`). The labels are kept
 * short so the row reads well at the modal's default width.
 *
 * Labels are getters (not values captured at module load) so they
 * resolve against the *current* language each time the modal renders.
 */
export const LOGIN_PHASES: PhaseSpec[] = [
  {
    id: "spawning",
    get label() {
      return i18n.t("loginPhases.spawning", { ns: "accounts" });
    },
  },
  {
    id: "waiting_for_browser",
    get label() {
      return i18n.t("loginPhases.waitingForBrowser", { ns: "accounts" });
    },
  },
  {
    id: "reading_blob",
    get label() {
      return i18n.t("loginPhases.readingBlob", { ns: "accounts" });
    },
  },
  {
    id: "fetching_profile",
    get label() {
      return i18n.t("loginPhases.fetchingProfile", { ns: "accounts" });
    },
  },
  {
    id: "verifying_identity",
    get label() {
      return i18n.t("loginPhases.verifyingIdentity", { ns: "accounts" });
    },
  },
  {
    id: "persisting",
    get label() {
      return i18n.t("loginPhases.persisting", { ns: "accounts" });
    },
  },
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
