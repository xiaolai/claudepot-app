import type { ReactNode } from "react";
import type { RunningOpInfo } from "../../types";
import type { PhaseSpec } from "./OperationProgressModal";

/**
 * Phase ids + labels emitted by
 * `claudepot_core::session_move::move_session_with_progress`. The
 * S-prefix is intentionally distinct from project-move's P-prefix so
 * mixed event streams can be filtered cleanly.
 */
export const SESSION_MOVE_PHASES: PhaseSpec[] = [
  { id: "S1", label: "Rewriting primary transcript" },
  { id: "S2", label: "Moving sidecar dirs" },
  { id: "S3", label: "Updating history.jsonl" },
  { id: "S4", label: "Clearing .claude.json pointers" },
  { id: "S5", label: "Cleaning up source dir" },
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
      <dt>Transcript lines rewritten</dt>
      <dd>{r.jsonlLinesRewritten}</dd>
      {r.subagentFilesMoved > 0 && (
        <>
          <dt>Subagent files moved</dt>
          <dd>{r.subagentFilesMoved}</dd>
        </>
      )}
      {r.remoteAgentFilesMoved > 0 && (
        <>
          <dt>Remote-agent files moved</dt>
          <dd>{r.remoteAgentFilesMoved}</dd>
        </>
      )}
      <dt>History entries followed</dt>
      <dd>
        {r.historyEntriesMoved}
        {r.historyEntriesUnmapped > 0 && (
          <span style={{ color: "var(--fg-faint)" }}>
            {" · "}
            {r.historyEntriesUnmapped} stayed (pre-sessionId)
          </span>
        )}
      </dd>
      {r.claudeJsonPointersCleared > 0 && (
        <>
          <dt>
            <code className="mono">.claude.json</code> pointers cleared
          </dt>
          <dd>{r.claudeJsonPointersCleared}</dd>
        </>
      )}
      {r.sourceDirRemoved && (
        <>
          <dt>Source project dir</dt>
          <dd>removed (was empty)</dd>
        </>
      )}
    </dl>
  );
}
