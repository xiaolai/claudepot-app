import type { ReactNode } from "react";
import { Trans } from "react-i18next";
import { i18n } from "../../lib/i18n";
import type { RunningOpInfo } from "../../types";
import type { PhaseSpec } from "./OperationProgressModal";

/** Namespace-bound t — language still resolves at call time. */
const t = i18n.getFixedT(null, "projects");

/**
 * Phase ids + labels emitted by `claudepot_core::project::move_project`
 * (see `crates/claudepot-core/src/project.rs` — one entry per
 * `sink.phase("Pn", …)` call site). Keep the labels short so the row
 * reads well at the modal's default width.
 *
 * `label` is a getter so the lookup happens where the row is rendered,
 * not at module load — a language switch re-renders the consuming
 * modal and re-reads the getter. The `{ id, label }` shape is shared
 * (`PhaseSpec`) with out-of-scope consumers and must not change.
 */
export const PROJECT_MOVE_PHASES: PhaseSpec[] = [
  { id: "P3", get label() { return t("rename.phaseP3"); } },
  { id: "P4", get label() { return t("rename.phaseP4"); } },
  { id: "P5", get label() { return t("rename.phaseP5"); } },
  { id: "P6", get label() { return t("rename.phaseP6"); } },
  { id: "P7", get label() { return t("rename.phaseP7"); } },
  { id: "P8", get label() { return t("rename.phaseP8"); } },
  { id: "P9", get label() { return t("rename.phaseP9"); } },
  { id: "P10", get label() { return t("rename.phaseP10"); } },
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
      {result.actual_dir_moved && <li>{t("rename.resultSourceMoved")}</li>}
      {result.cc_dir_renamed && (
        <li>
          {t("rename.resultCcRenamed", {
            count: result.jsonl_files_scanned,
            modified: result.jsonl_files_modified,
          })}
        </li>
      )}
      {result.memory_dir_moved && <li>{t("rename.resultMemoryMoved")}</li>}
      {result.plugin_bindings_rewritten > 0 && (
        <li>
          {t("rename.resultBindings", {
            count: result.plugin_bindings_rewritten,
          })}
        </li>
      )}
      {result.config_had_collision && result.config_snapshot_path && (
        <li>
          <Trans
            ns="projects"
            i18nKey="rename.resultPreserved"
            components={{
              p: (
                <code className="mono small">
                  {result.config_snapshot_path}
                </code>
              ),
            }}
          />
        </li>
      )}
      {result.warnings.length > 0 && (
        <li className="muted small">
          {t("rename.resultWarnings")}
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
