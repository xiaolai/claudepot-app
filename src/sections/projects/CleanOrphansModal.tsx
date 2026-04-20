import { useCallback, useEffect, useRef, useState } from "react";
import { Icon } from "../../components/Icon";
import { api } from "../../api";
import { CopyButton } from "../../components/CopyButton";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import type {
  CleanPreview,
  CleanResult,
  OperationProgressEvent,
  ProjectInfo,
} from "../../types";
import { formatSize } from "./format";

type State =
  | { kind: "loading" }
  | { kind: "preview"; data: CleanPreview }
  | { kind: "running"; opId: string; phase: string; done: number; total: number }
  | { kind: "done"; result: CleanResult }
  | { kind: "error"; message: string };

/**
 * Confirm + execute dialog for `project clean`. Subscribes to
 * `op-progress::<opId>` once the clean task is started so the
 * user sees live "N of M" feedback instead of a mysterious spinner.
 *
 * Lifecycle:
 *   1. loading  — fetch preview on mount. User sees a skeleton.
 *   2. preview  — list of orphan candidates + unreachable skip note.
 *                 Confirm enabled only when the orphan count > 0.
 *   3. running  — progress bar driven by sub_progress events. The
 *                 backend emits two phases: `batch-sibling` (single
 *                 pass through history.jsonl + ~/.claude.json) and
 *                 `remove-dirs` (per-orphan remove_dir_all). We
 *                 surface the currently-active phase's progress.
 *   4. done     — counters panel + recovery snapshot paths.
 *   5. error    — backend error (journal gate, lock race, etc.).
 *
 * The dialog is dismissable in every state EXCEPT running. Running
 * is non-dismissable because the backend is holding the clean lock
 * and actively mutating disk; abandoning mid-run would leave
 * subsequent starts with a stale lock to break.
 */
export function CleanOrphansModal({
  onClose,
  onDone,
}: {
  onClose: () => void;
  onDone: (result: CleanResult) => void;
}) {
  const [state, setState] = useState<State>({ kind: "loading" });
  const headingId = useRef(
    `clean-heading-${Math.random().toString(36).slice(2, 9)}`,
  );
  const trapRef = useFocusTrap<HTMLDivElement>();
  const firedTerminal = useRef(false);

  const loadPreview = useCallback(() => {
    setState({ kind: "loading" });
    firedTerminal.current = false;
    api
      .projectCleanPreview()
      .then((data) => setState({ kind: "preview", data }))
      .catch((e) => setState({ kind: "error", message: String(e) }));
  }, []);

  useEffect(() => {
    loadPreview();
  }, [loadPreview]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && state.kind !== "running") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, state.kind]);

  // Subscribe to progress events only while a clean is running.
  const channel =
    state.kind === "running" ? `op-progress::${state.opId}` : null;
  const opIdRef = useRef<string | null>(null);

  const handleEvent = useCallback(
    (event: { payload: OperationProgressEvent }) => {
      const ev = event.payload;
      if (ev.op_id !== opIdRef.current) return;

      if (ev.phase === "op") {
        if (firedTerminal.current) return;
        firedTerminal.current = true;
        const isComplete = ev.status === "complete";
        api
          .projectCleanStatus(ev.op_id)
          .then((info) => {
            if (isComplete && info?.clean_result) {
              setState({ kind: "done", result: info.clean_result });
              onDone(info.clean_result);
            } else if (isComplete) {
              // Terminal complete but poll missed the result — synthesize
              // an empty one so the UI doesn't wedge.
              const empty: CleanResult = {
                orphans_found: 0,
                orphans_removed: 0,
                orphans_skipped_live: 0,
                unreachable_skipped: 0,
                bytes_freed: 0,
                claude_json_entries_removed: 0,
                history_lines_removed: 0,
                claudepot_artifacts_removed: 0,
                snapshot_paths: [],
                protected_paths_skipped: 0,
              };
              setState({ kind: "done", result: empty });
              onDone(empty);
            } else {
              setState({
                kind: "error",
                message: ev.detail ?? info?.last_error ?? "clean failed",
              });
            }
          })
          .catch(() => {
            setState({
              kind: "error",
              message: ev.detail ?? "clean failed (unreachable status)",
            });
          });
        return;
      }

      // Phase + sub_progress updates. Only advance the surfaced phase
      // when we actually get a sub_progress tuple; pure status events
      // ("batch-sibling complete" without done/total) flip the phase
      // label and reset the counter.
      if (typeof ev.done === "number" && typeof ev.total === "number") {
        setState((prev) =>
          prev.kind === "running"
            ? {
                kind: "running",
                opId: prev.opId,
                phase: ev.phase,
                done: ev.done!,
                total: ev.total!,
              }
            : prev,
        );
      } else if (ev.status === "running") {
        setState((prev) =>
          prev.kind === "running"
            ? { ...prev, phase: ev.phase, done: 0, total: prev.total }
            : prev,
        );
      }
    },
    [onDone],
  );

  useTauriEvent<OperationProgressEvent>(channel, handleEvent);

  const runClean = () => {
    firedTerminal.current = false;
    api
      .projectCleanStart()
      .then((opId) => {
        opIdRef.current = opId;
        setState({
          kind: "running",
          opId,
          phase: "batch-sibling",
          done: 0,
          total: 0,
        });
      })
      .catch((e) => setState({ kind: "error", message: String(e) }));
  };

  const closeSafe = () => {
    if (state.kind === "running") return;
    onClose();
  };

  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onClick={(e) => {
        if (e.target === e.currentTarget) closeSafe();
      }}
    >
      <div
        ref={trapRef}
        className="modal clean-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby={headingId.current}
      >
        <h2 id={headingId.current}>Clean project data</h2>

        <div className="modal-body">
          {state.kind === "loading" && <SkeletonPreview />}

          {state.kind === "preview" && (
            <Preview data={state.data} onRefresh={loadPreview} />
          )}

          {state.kind === "running" && (
            <RunningView
              phase={state.phase}
              done={state.done}
              total={state.total}
            />
          )}

          {state.kind === "done" && <Result result={state.result} />}

          {state.kind === "error" && (
            <div className="clean-error" role="alert">
              <Icon name="alert-triangle" size={14} />
              <div>
                <strong>Couldn't clean.</strong>
                <p className="mono small">{state.message}</p>
                <p className="muted small">
                  If pending rename journals are blocking, resolve them
                  in the Repair view first.
                </p>
              </div>
            </div>
          )}
        </div>

        <div className="modal-actions">
          {state.kind === "done" ? (
            <button
              type="button"
              className="btn primary"
              onClick={closeSafe}
              autoFocus
            >
              Close
            </button>
          ) : (
            <>
              <button
                type="button"
                className="btn"
                onClick={closeSafe}
                disabled={state.kind === "running"}
                title={
                  state.kind === "running"
                    ? "Can't cancel mid-run — the backend is holding the clean lock"
                    : undefined
                }
              >
                {state.kind === "running"
                  ? "Running…"
                  : state.kind === "error"
                    ? "Close"
                    : "Cancel"}
              </button>
              <button
                type="button"
                className="btn danger primary"
                disabled={
                  !(state.kind === "preview" && state.data.orphans_found > 0)
                }
                onClick={runClean}
              >
                <Icon name="trash-2" size={13} />
                {state.kind === "preview" && state.data.orphans_found > 0
                  ? `Remove ${state.data.orphans_found} project${
                      state.data.orphans_found === 1 ? "" : "s"
                    }`
                  : "Remove"}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function SkeletonPreview() {
  return (
    <div className="skeleton-container">
      <div className="skeleton skeleton-header" />
      <div className="skeleton skeleton-card" />
    </div>
  );
}

function Preview({
  data,
  onRefresh,
}: {
  data: CleanPreview;
  onRefresh: () => void;
}) {
  if (data.orphans_found === 0 && data.unreachable_skipped === 0) {
    return (
      <div className="clean-empty">
        <p>Nothing to clean.</p>
        <p className="muted small">
          Every CC project dir maps to a source path that exists. Good.
        </p>
      </div>
    );
  }

  return (
    <>
      <p className="clean-summary">
        <strong>{data.orphans_found}</strong> project
        {data.orphans_found === 1 ? "" : "s"} will be removed (
        {formatSize(data.total_bytes)}).
      </p>

      {data.unreachable_skipped > 0 && (
        <div className="clean-unreachable" role="status">
          <Icon name="wifi-off" size={14} />
          <span>
            <strong>{data.unreachable_skipped}</strong> project
            {data.unreachable_skipped === 1 ? "" : "s"} with unreachable
            source paths will be left alone (drive unmounted or
            permission denied).{" "}
            <button
              type="button"
              className="link-btn"
              onClick={onRefresh}
              title="Re-run the preview"
            >
              Refresh
            </button>
          </span>
        </div>
      )}

      {data.orphans_found > 0 && (
        <ul className="clean-orphan-list" aria-label="Projects to be removed">
          {data.orphans.map((p) => (
            <OrphanRow key={p.sanitized_name} info={p} />
          ))}
        </ul>
      )}

      <p className="muted small clean-disclaimer">
        Also prunes matching entries in <code>~/.claude.json</code> and{" "}
        <code>history.jsonl</code>. Recovery snapshots are written before
        anything is deleted.
      </p>

      {data.protected_count > 0 && (
        <p className="muted small clean-disclaimer">
          <strong>{data.protected_count}</strong> of these{" "}
          {data.protected_count === 1 ? "is" : "are"} on your protected list
          — its CC artifact directory will be removed, but{" "}
          <code>~/.claude.json</code> and <code>history.jsonl</code> entries
          for that path will be preserved.
        </p>
      )}
    </>
  );
}

function OrphanRow({ info }: { info: ProjectInfo }) {
  return (
    <li className="clean-orphan-row">
      <div className="clean-orphan-main">
        <span className="mono small selectable" title={info.original_path}>
          {info.original_path}
        </span>
        <span className="muted small">
          {info.session_count} session{info.session_count === 1 ? "" : "s"} ·{" "}
          {formatSize(info.total_size_bytes)}
        </span>
      </div>
      {info.is_empty && (
        <span className="project-tag empty" title="empty project dir">
          <Icon name="circle-dashed" size={11} /> empty
        </span>
      )}
    </li>
  );
}

const PHASE_LABEL: Record<string, string> = {
  "batch-sibling": "Rewriting ~/.claude.json and history.jsonl",
  "remove-dirs": "Removing project directories",
};

function RunningView({
  phase,
  done,
  total,
}: {
  phase: string;
  done: number;
  total: number;
}) {
  const label = PHASE_LABEL[phase] ?? "Cleaning";
  const pct =
    total > 0 ? Math.round((Math.min(done, total) / total) * 100) : 0;
  return (
    <div className="clean-running" role="status" aria-live="polite">
      <p>{label}…</p>
      {total > 0 ? (
        <>
          <div className="clean-progress-track" aria-hidden="true">
            <div
              className="clean-progress-fill"
              style={{ width: `${pct}%` }}
            />
          </div>
          <p className="muted small">
            {done} of {total}
            {phase === "remove-dirs" ? " projects" : " steps"}
          </p>
        </>
      ) : (
        <div className="clean-spinner" aria-hidden="true" />
      )}
    </div>
  );
}

function Result({ result }: { result: CleanResult }) {
  return (
    <>
      <p className="clean-summary">
        Removed <strong>{result.orphans_removed}</strong> project
        {result.orphans_removed === 1 ? "" : "s"}, freed{" "}
        <strong>{formatSize(result.bytes_freed)}</strong>.
      </p>

      {result.orphans_skipped_live > 0 && (
        <div className="clean-unreachable" role="status">
          <Icon name="alert-triangle" size={14} />
          <span>
            <strong>{result.orphans_skipped_live}</strong> project
            {result.orphans_skipped_live === 1 ? "" : "s"} skipped because a
            live Claude Code session was detected. Quit the session and
            re-run.
          </span>
        </div>
      )}

      <ul className="clean-result-list">
        {result.claude_json_entries_removed > 0 && (
          <li>
            Pruned {result.claude_json_entries_removed} entr
            {result.claude_json_entries_removed === 1 ? "y" : "ies"} from{" "}
            <code>~/.claude.json</code>
          </li>
        )}
        {result.history_lines_removed > 0 && (
          <li>
            Removed {result.history_lines_removed} line
            {result.history_lines_removed === 1 ? "" : "s"} from{" "}
            <code>history.jsonl</code>
          </li>
        )}
        {result.claudepot_artifacts_removed > 0 && (
          <li>
            Removed {result.claudepot_artifacts_removed} stale claudepot
            artifact{result.claudepot_artifacts_removed === 1 ? "" : "s"}
          </li>
        )}
        {result.protected_paths_skipped > 0 && (
          <li>
            Preserved sibling state for {result.protected_paths_skipped}{" "}
            protected path{result.protected_paths_skipped === 1 ? "" : "s"}{" "}
            (CC artifact dir{result.protected_paths_skipped === 1 ? "" : "s"}{" "}
            still removed)
          </li>
        )}
      </ul>

      {result.snapshot_paths.length > 0 && (
        <div className="clean-snapshots">
          <div className="field-label">Recovery snapshots</div>
          <p className="muted small">
            Saved before anything was deleted. Copy a path and open it with
            a JSON viewer to restore.
          </p>
          <ul className="clean-snapshot-list">
            {result.snapshot_paths.map((p) => (
              <li key={p}>
                <span className="mono small selectable">{p}</span>
                <CopyButton text={p} />
              </li>
            ))}
          </ul>
        </div>
      )}
    </>
  );
}

