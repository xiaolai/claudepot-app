import type { ReactNode } from "react";
import { i18n } from "../../lib/i18n";
import type { RunningOpInfo } from "../../types";
import type { PhaseSpec } from "./OperationProgressModal";

/** Namespace-bound t — language still resolves at call time. */
const t = i18n.getFixedT(null, "projects");

/**
 * Phase ids + labels emitted by
 * `claudepot_core::session_move::move_session_with_progress`. The
 * S-prefix is intentionally distinct from project-move's P-prefix so
 * mixed event streams can be filtered cleanly.
 *
 * `label` is a getter so the lookup happens where the row is rendered,
 * not at module load — see `projectMoveProgress.tsx`.
 */
export const SESSION_MOVE_PHASES: PhaseSpec[] = [
  { id: "S1", get label() { return t("move.phaseS1"); } },
  { id: "S2", get label() { return t("move.phaseS2"); } },
  { id: "S3", get label() { return t("move.phaseS3"); } },
  { id: "S4", get label() { return t("move.phaseS4"); } },
  { id: "S5", get label() { return t("move.phaseS5"); } },
];

/**
 * Render the success-state body for a session move. Uses the
 * `MoveSessionReport` mirror surfaced as `info.session_move_result` —
 * same shape the legacy synchronous `sessionMove` IPC returned.
 */
export function renderSessionMoveResult(info: RunningOpInfo | null): ReactNode {
  const r = info?.session_move_result;
  if (!r) return null;
  return (
    <dl className="detail-grid">
      <dt>{t("move.resultLines")}</dt>
      <dd>{r.jsonlLinesRewritten}</dd>
      {r.subagentFilesMoved > 0 && (
        <>
          <dt>{t("move.resultSubagent")}</dt>
          <dd>{r.subagentFilesMoved}</dd>
        </>
      )}
      {r.remoteAgentFilesMoved > 0 && (
        <>
          <dt>{t("move.resultRemote")}</dt>
          <dd>{r.remoteAgentFilesMoved}</dd>
        </>
      )}
      <dt>{t("move.resultHistory")}</dt>
      <dd>
        {r.historyEntriesMoved}
        {r.historyEntriesUnmapped > 0 && (
          <span style={{ color: "var(--fg-faint)" }}>
            {" · "}
            {t("move.resultUnmapped", { n: r.historyEntriesUnmapped })}
          </span>
        )}
      </dd>
      {r.claudeJsonPointersCleared > 0 && (
        <>
          <dt>
            <code className="mono">.claude.json</code> {t("move.resultPointers")}
          </dt>
          <dd>{r.claudeJsonPointersCleared}</dd>
        </>
      )}
      {r.sourceDirRemoved && (
        <>
          <dt>{t("move.resultSourceDir")}</dt>
          <dd>{t("move.resultRemoved")}</dd>
        </>
      )}
    </dl>
  );
}
