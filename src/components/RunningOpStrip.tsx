import type { RunningOpInfo } from "../types";

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

function labelFor(op: RunningOpInfo): string {
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

/**
 * Bottom-of-window strip showing background ops. Visible only when
 * ops are actually running; disappears entirely when empty so it
 * doesn't eat screen real estate. Clicking an op re-opens the
 * corresponding progress modal (the parent component wires that).
 */
export function RunningOpStrip({
  ops,
  onReopen,
}: {
  ops: RunningOpInfo[];
  onReopen: (opId: string) => void;
}) {
  const running = ops.filter((o) => o.status === "running");
  if (running.length === 0) return null;

  return (
    <aside
      className="running-op-strip"
      aria-label="Background operations"
      role="status"
    >
      {running.map((op) => (
        <button
          key={op.op_id}
          type="button"
          className="running-op-strip-item"
          title="Re-open progress modal"
          onClick={() => onReopen(op.op_id)}
        >
          <span className="running-op-spinner" aria-hidden="true" />
          <span className="running-op-label">{labelFor(op)}</span>
        </button>
      ))}
    </aside>
  );
}
