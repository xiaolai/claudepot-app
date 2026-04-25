import type { ReactNode } from "react";
import type { RunningOpInfo } from "../../types";
import type { PhaseSpec } from "./OperationProgressModal";

/**
 * Phase ids + labels emitted by `claudepot_core::project::move_project`
 * (see `crates/claudepot-core/src/project.rs` — one entry per
 * `sink.phase("Pn", …)` call site). Keep the labels short so the row
 * reads well at the modal's default width.
 */
export const PROJECT_MOVE_PHASES: PhaseSpec[] = [
  { id: "P3", label: "Moving source directory" },
  { id: "P4", label: "Renaming Claude Code project" },
  { id: "P5", label: "Updating history.jsonl" },
  { id: "P6", label: "Rewriting session transcripts" },
  { id: "P7", label: "Updating Claude Code config" },
  { id: "P8", label: "Moving auto-memory directory" },
  { id: "P9", label: "Updating project settings" },
];

/**
 * Render the success-state body for a project move. Mirrors the inline
 * panel that used to live inside `OperationProgressModal`. Reads the
 * structured summary from `info.move_result`.
 */
export function renderProjectMoveResult(info: RunningOpInfo | null): ReactNode {
  const result = info?.move_result;
  if (!result) return null;
  return (
    <ul className="op-terminal-detail">
      {result.actual_dir_moved && <li>Source directory moved.</li>}
      {result.cc_dir_renamed && (
        <li>
          CC project dir renamed; {result.jsonl_files_modified} of{" "}
          {result.jsonl_files_scanned} jsonl file
          {result.jsonl_files_scanned === 1 ? "" : "s"} rewritten.
        </li>
      )}
      {result.memory_dir_moved && <li>Auto-memory directory moved.</li>}
      {result.config_had_collision && result.config_snapshot_path && (
        <li>
          Pre-existing data preserved at{" "}
          <code className="mono small">{result.config_snapshot_path}</code>.
          Retained 30 days.
        </li>
      )}
      {result.warnings.length > 0 && (
        <li className="muted small">
          Warnings:
          <ul>
            {result.warnings.map((w, i) => (
              <li key={i}>{w}</li>
            ))}
          </ul>
        </li>
      )}
    </ul>
  );
}
