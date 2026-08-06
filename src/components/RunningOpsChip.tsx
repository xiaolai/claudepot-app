import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Glyph } from "./primitives/Glyph";
import { NF } from "../icons";
import { i18n } from "../lib/i18n";
import { basename } from "../lib/paths";
import { usePopoverDismiss } from "../hooks/usePopoverDismiss";
import type { RunningOpInfo } from "../types";

/**
 * Status-bar chip for in-flight long-running ops (verify_all, project
 * rename, repair resume/rollback, session prune/slim/share, account
 * login/register). Replaces the always-dark `RunningOpStrip` HUD
 * that floated above the content pane.
 *
 * Closed shape: `● 1 op` with a pulsing accent dot. Click → popover
 * lists every running op with its phase + sub-progress; clicking a
 * row re-opens the corresponding `OperationProgressModal` via the
 * caller-supplied `onReopen`.
 *
 * Renders nothing when there are no running ops — the bar layout
 * collapses around it like every other render-if-nonzero segment.
 */
export function RunningOpsChip({
  ops,
  onReopen,
}: {
  ops: RunningOpInfo[];
  onReopen: (opId: string) => void;
}) {
  const { t } = useTranslation("components");
  const running = ops.filter((o) => o.status === "running");
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  // Close on outside click + Escape — shared popover-dismiss hook.
  usePopoverDismiss(rootRef, open, () => setOpen(false));

  // Auto-close when the last op finishes — otherwise the popover
  // would render against an empty list. The chip itself disappears
  // on the same tick because `running.length === 0` short-circuits
  // below.
  useEffect(() => {
    if (running.length === 0 && open) setOpen(false);
  }, [running.length, open]);

  if (running.length === 0) return null;

  const label = t("chips.ops", { count: running.length });

  return (
    <div ref={rootRef} style={{ position: "relative" }}>
      <button
        type="button"
        className="statusbar-chip"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={t("chips.opsAria", { count: running.length })}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="statusbar-chip-pulse" aria-hidden="true" />
        <span>{label}</span>
      </button>

      {open && (
        <div
          className="statusbar-chip-popover"
          role="menu"
          aria-label={t("chips.opsGroup")}
        >
          <div className="statusbar-chip-popover-header">
            {t("chips.opsGroup")}
          </div>
          <div className="statusbar-chip-popover-list">
            {running.map((op) => (
              <button
                key={op.op_id}
                type="button"
                role="menuitem"
                className="statusbar-chip-popover-item"
                title={t("chips.opsReopen")}
                onClick={() => {
                  onReopen(op.op_id);
                  setOpen(false);
                }}
              >
                <span className="statusbar-chip-pulse" aria-hidden="true" />
                <span className="statusbar-chip-popover-label">
                  {labelFor(op)}
                </span>
                <Glyph
                  g={NF.openExternal}
                  color="var(--fg-faint)"
                  style={{ fontSize: "var(--fs-2xs)" }}
                />
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

/**
 * `verb` and `labelFor` are plain functions, not components — they
 * read the global i18n instance. `RunningOpsChip` subscribes via
 * `useTranslation`, so a language switch re-renders it and re-invokes
 * these.
 */
const NS = { ns: "components" } as const;

function verb(kind: RunningOpInfo["kind"]): string {
  switch (kind) {
    case "repair_resume":
      return i18n.t("ops.resuming", NS);
    case "repair_rollback":
      return i18n.t("ops.rollingBack", NS);
    case "move_project":
      return i18n.t("ops.renaming", NS);
    case "clean_projects":
      return i18n.t("ops.cleaning", NS);
    case "session_prune":
      return i18n.t("ops.pruning", NS);
    case "session_slim":
      return i18n.t("ops.slimming", NS);
    case "session_share":
      return i18n.t("ops.sharing", NS);
    case "session_move":
      return i18n.t("ops.movingSession", NS);
    case "account_login":
      return i18n.t("ops.loggingIn", NS);
    case "account_register":
      return i18n.t("ops.addingAccount", NS);
    case "verify_all":
      return i18n.t("ops.verifying", NS);
  }
}

export function labelFor(op: RunningOpInfo): string {
  if (op.kind === "clean_projects") {
    if (op.current_phase && op.sub_progress) {
      const [done, total] = op.sub_progress;
      return i18n.t("ops.cleaningProjectsProgress", { ...NS, done, total });
    }
    return i18n.t("ops.cleaningProjects", NS);
  }
  if (op.kind === "session_prune") {
    return op.sub_progress
      ? i18n.t("ops.pruningSessionsProgress", {
          ...NS,
          done: op.sub_progress[0],
          total: op.sub_progress[1],
        })
      : i18n.t("ops.pruningSessions", NS);
  }
  if (op.kind === "session_slim") {
    const file = basename(op.old_path) || i18n.t("ops.sessionFallback", NS);
    return op.current_phase
      ? i18n.t("ops.slimmingFilePhase", {
          ...NS,
          file,
          phase: op.current_phase,
        })
      : i18n.t("ops.slimmingFile", { ...NS, file });
  }
  if (op.kind === "session_share") {
    return op.current_phase
      ? i18n.t("ops.sharingPhase", { ...NS, phase: op.current_phase })
      : i18n.t("ops.sharingSession", NS);
  }
  const base = i18n.t("ops.rename", {
    ...NS,
    verb: verb(op.kind),
    from: basename(op.old_path),
    to: basename(op.new_path),
  });
  if (op.current_phase && op.sub_progress) {
    const [done, total] = op.sub_progress;
    return i18n.t("ops.renameFiles", {
      ...NS,
      base,
      phase: op.current_phase,
      done,
      total,
    });
  }
  if (op.current_phase) {
    return i18n.t("ops.renamePhase", { ...NS, base, phase: op.current_phase });
  }
  return base;
}
