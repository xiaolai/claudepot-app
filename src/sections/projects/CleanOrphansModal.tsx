import { useCallback, useEffect, useRef, useState } from "react";
import { Trash, Warning, WifiSlash, CircleDashed } from "@phosphor-icons/react";
import { api } from "../../api";
import { CopyButton } from "../../components/CopyButton";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import type { CleanPreview, CleanResult, ProjectInfo } from "../../types";

type State =
  | { kind: "loading" }
  | { kind: "preview"; data: CleanPreview }
  | { kind: "running" }
  | { kind: "done"; result: CleanResult }
  | { kind: "error"; message: string };

/**
 * Confirm + execute dialog for `project clean`. Three-phase flow:
 *
 *   1. loading  — fetch preview on mount. User sees a skeleton.
 *   2. preview  — list of orphan candidates + unreachable skip note.
 *                 Confirm button enabled only when there is at
 *                 least one candidate.
 *   3. running  — disabled spinner-style state while the backend
 *                 holds the clean lock and deletes.
 *   4. done     — counters panel with any recovery snapshot paths.
 *   5. error    — backend error (e.g. journal gate or lock race).
 *
 * The dialog closes on Escape or backdrop click in preview / done /
 * error states. Running is NON-dismissable because the core is
 * already modifying disk — interrupting would leave state in a
 * half-cleaned form.
 */
export function CleanOrphansModal({
  onClose,
  onDone,
}: {
  onClose: () => void;
  /** Fires after a successful clean so the parent can refresh the list
   *  and surface a "X skipped due to live session" toast if applicable. */
  onDone: (result: CleanResult) => void;
}) {
  const [state, setState] = useState<State>({ kind: "loading" });
  const headingId = useRef(
    `clean-heading-${Math.random().toString(36).slice(2, 9)}`,
  );
  const trapRef = useFocusTrap<HTMLDivElement>();

  const loadPreview = useCallback(() => {
    setState({ kind: "loading" });
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

  const runClean = () => {
    setState({ kind: "running" });
    api
      .projectCleanExecute()
      .then((result) => {
        setState({ kind: "done", result });
        onDone(result);
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
            <div className="clean-running" role="status">
              <p>Cleaning…</p>
              <p className="muted small">
                Removing project dirs, pruning sibling state, writing
                recovery snapshots.
              </p>
            </div>
          )}

          {state.kind === "done" && <Result result={state.result} />}

          {state.kind === "error" && (
            <div className="clean-error" role="alert">
              <Warning size={16} weight="bold" />
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
              className="primary"
              onClick={closeSafe}
              autoFocus
            >
              Close
            </button>
          ) : (
            <>
              <button
                type="button"
                onClick={closeSafe}
                disabled={state.kind === "running"}
              >
                {state.kind === "error" ? "Close" : "Cancel"}
              </button>
              <button
                type="button"
                className="danger primary"
                disabled={
                  !(state.kind === "preview" && state.data.orphans_found > 0)
                }
                onClick={runClean}
              >
                <Trash size={13} weight="light" />
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
          <WifiSlash size={14} weight="light" />
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
          <CircleDashed size={11} weight="bold" /> empty
        </span>
      )}
    </li>
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
          <Warning size={14} weight="light" />
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

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}
