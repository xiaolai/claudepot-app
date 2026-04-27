import { useEffect, useRef, useState } from "react";
import { Glyph } from "./primitives/Glyph";
import { NF } from "../icons";
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
  const running = ops.filter((o) => o.status === "running");
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  // Close on outside click + Escape — same shape as the sidebar
  // target switcher's popover. The 0ms timeout defers wiring past
  // the click that opened the popover so it doesn't re-close on the
  // same event tick.
  useEffect(() => {
    if (!open) return;
    const onDocClick = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    const t = window.setTimeout(() => {
      document.addEventListener("mousedown", onDocClick);
    }, 0);
    window.addEventListener("keydown", onKey);
    return () => {
      window.clearTimeout(t);
      document.removeEventListener("mousedown", onDocClick);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  // Auto-close when the last op finishes — otherwise the popover
  // would render against an empty list. The chip itself disappears
  // on the same tick because `running.length === 0` short-circuits
  // below.
  useEffect(() => {
    if (running.length === 0 && open) setOpen(false);
  }, [running.length, open]);

  if (running.length === 0) return null;

  const label =
    running.length === 1
      ? "1 op"
      : `${running.length} ops`;

  return (
    <div ref={rootRef} style={{ position: "relative" }}>
      <button
        type="button"
        className="statusbar-chip"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`${running.length} background operation${running.length === 1 ? "" : "s"} running. Click to view details.`}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="statusbar-chip-pulse" aria-hidden="true" />
        <span>{label}</span>
      </button>

      {open && (
        <div
          className="statusbar-chip-popover"
          role="menu"
          aria-label="Background operations"
        >
          <div className="statusbar-chip-popover-header">
            Background operations
          </div>
          <div className="statusbar-chip-popover-list">
            {running.map((op) => (
              <button
                key={op.op_id}
                type="button"
                role="menuitem"
                className="statusbar-chip-popover-item"
                title="Re-open progress modal"
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

function verb(kind: RunningOpInfo["kind"]): string {
  switch (kind) {
    case "repair_resume":
      return "Resuming";
    case "repair_rollback":
      return "Rolling back";
    case "move_project":
      return "Renaming";
    case "clean_projects":
      return "Cleaning";
    case "session_prune":
      return "Pruning";
    case "session_slim":
      return "Slimming";
    case "session_share":
      return "Sharing";
    case "session_move":
      return "Moving session";
    case "account_login":
      return "Logging in";
    case "account_register":
      return "Adding account";
    case "verify_all":
      return "Verifying";
  }
}

export function labelFor(op: RunningOpInfo): string {
  if (op.kind === "clean_projects") {
    if (op.current_phase && op.sub_progress) {
      const [done, total] = op.sub_progress;
      return `Cleaning projects (${done}/${total})`;
    }
    return "Cleaning projects";
  }
  if (op.kind === "session_prune") {
    const suffix = op.sub_progress
      ? ` (${op.sub_progress[0]}/${op.sub_progress[1]})`
      : "";
    return `Pruning sessions${suffix}`;
  }
  if (op.kind === "session_slim") {
    const file = basename(op.old_path) || "session";
    return op.current_phase
      ? `Slimming ${file} (${op.current_phase})`
      : `Slimming ${file}`;
  }
  if (op.kind === "session_share") {
    return op.current_phase
      ? `Sharing (${op.current_phase})`
      : "Sharing session";
  }
  const base = `${verb(op.kind)} ${basename(op.old_path)} → ${basename(op.new_path)}`;
  if (op.current_phase && op.sub_progress) {
    const [done, total] = op.sub_progress;
    return `${base} (${op.current_phase}: ${done}/${total} files)`;
  }
  if (op.current_phase) return `${base} (${op.current_phase})`;
  return base;
}

function basename(path: string): string {
  return path.split("/").filter(Boolean).pop() ?? path;
}
