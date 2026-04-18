import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { FolderOpen, AlertTriangle } from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../../api";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import { DRY_RUN_SUPERSEDED, type DryRunPlan, type MoveArgs } from "../../types";

const DEBOUNCE_MS = 300;

type PreviewState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ok"; plan: DryRunPlan }
  | { kind: "error"; message: string };

type CollisionPolicy = "none" | "merge" | "overwrite";

/**
 * Rename modal. Per plan §7.1:
 * - Path text input is the primary authority. "Browse parent…" helps
 *   but writes into the same field so case-only renames and arbitrary
 *   basenames still work.
 * - Dry-run preview is debounced (300ms) and re-requested on every
 *   change of inputs that affect the plan (new path, collision policy,
 *   flags).
 * - Danger zone visually separates `--force` and
 *   `--ignore-pending-journals` from the collision radio. Each has
 *   explicit consequence copy.
 * - Submit is explicitly not a safety claim — copy below the button
 *   says so verbatim.
 *
 * Submit itself is stubbed in this step; Step 6 wires it to
 * `project_move_start`.
 */
export function RenameProjectModal({
  oldPath,
  onClose,
  onSubmit,
}: {
  oldPath: string;
  onClose: () => void;
  /** Called when the user confirms. Parent performs the execution. */
  onSubmit: (args: MoveArgs) => void;
}) {
  const [newPath, setNewPath] = useState<string>(oldPath);
  const [collision, setCollision] = useState<CollisionPolicy>("none");
  const [force, setForce] = useState(false);
  const [ignorePending, setIgnorePending] = useState(false);
  const [noMove, setNoMove] = useState(false);
  const [preview, setPreview] = useState<PreviewState>({ kind: "idle" });

  const headingId = useRef(
    `rename-heading-${Math.random().toString(36).slice(2, 9)}`,
  );
  const trapRef = useFocusTrap<HTMLDivElement>();

  // Used to drop stale preview responses: every keystroke increments
  // the token; on response we check ours still matches. Cheaper than
  // aborting Tauri invokes — which Tauri doesn't support anyway — and
  // it also cheaply drops responses that raced a later keystroke.
  const reqToken = useRef(0);

  const args: MoveArgs = useMemo(
    () => ({
      oldPath,
      newPath,
      noMove,
      merge: collision === "merge",
      overwrite: collision === "overwrite",
      force,
      ignorePendingJournals: ignorePending,
    }),
    [oldPath, newPath, noMove, collision, force, ignorePending],
  );

  const runPreview = useCallback(() => {
    if (!newPath.trim()) {
      // Audit M17: advance the token even on the empty-input branch.
      // Previously the token only incremented inside the non-empty
      // path, so if the user cleared the input while a request was
      // in flight, that in-flight response could still arrive and
      // repopulate the preview for an empty input (stale-data leak).
      ++reqToken.current;
      setPreview({ kind: "idle" });
      return;
    }
    const myToken = ++reqToken.current;
    setPreview({ kind: "loading" });
    // Send the token to the backend so it can short-circuit stale work
    // on its side too (plan §7.1). Monotonic + shared process-wide is
    // fine — the backend's DryRunRegistry uses fetch_max.
    api
      .projectMoveDryRun({ ...args, cancelToken: myToken })
      .then((plan) => {
        if (myToken !== reqToken.current) return; // stale
        setPreview({ kind: "ok", plan });
      })
      .catch((e) => {
        if (myToken !== reqToken.current) return;
        const msg = String(e);
        // Backend sentinel: it noticed we were superseded and bailed.
        // Leave the preview state as-is so the UI doesn't flash an
        // error — a newer call is already in flight.
        if (msg.includes(DRY_RUN_SUPERSEDED)) return;
        setPreview({ kind: "error", message: msg });
      });
  }, [args, newPath]);

  useEffect(() => {
    const handle = window.setTimeout(runPreview, DEBOUNCE_MS);
    return () => window.clearTimeout(handle);
  }, [runPreview]);

  // Audit M17: invalidate the last token when the modal closes so
  // any in-flight dry-run can't call setPreview after unmount.
  useEffect(() => {
    return () => {
      // Bumping the token past any in-flight call's value guarantees
      // the stale-response guard fails for responses that land post-unmount.
      reqToken.current += 1;
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const browseParent = async () => {
    try {
      const result = await openDialog({
        directory: true,
        multiple: false,
        title: "Choose parent folder",
      });
      if (typeof result === "string" && result) {
        const basename = currentBasename(newPath) || currentBasename(oldPath);
        setNewPath(basename ? `${result.replace(/\/$/, "")}/${basename}` : result);
      }
    } catch (e) {
      console.warn("browse dialog failed", e);
    }
  };

  const conflict = preview.kind === "ok" ? preview.plan.conflict : null;
  const conflictNeedsPolicy = Boolean(conflict) && collision === "none";
  const submitDisabled =
    !newPath.trim() ||
    newPath === oldPath ||
    preview.kind !== "ok" ||
    conflictNeedsPolicy;

  return (
    <div className="modal-backdrop" role="presentation">
      <div
        ref={trapRef}
        className="modal rename-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby={headingId.current}
      >
        <h2 id={headingId.current}>Rename project</h2>

        <div className="modal-body">
          <label className="field-label">Current path</label>
          <div className="mono small selectable muted">{oldPath}</div>

          <label className="field-label" htmlFor="rename-new-path">
            New path
          </label>
          <div className="field-row">
            <input
              id="rename-new-path"
              type="text"
              className="mono"
              value={newPath}
              spellCheck={false}
              autoCapitalize="off"
              autoComplete="off"
              // useFocusTrap picks this up via [autofocus]; the native
              // `autoFocus` attribute on top of that caused a double
              // focus-set on mount.
              onChange={(e) => setNewPath(e.target.value)}
            />
            <button
              type="button"
              className="icon-btn"
              title="Browse for parent folder"
              aria-label="Browse for parent folder"
              onClick={browseParent}
            >
              <FolderOpen />
            </button>
          </div>
          <p className="muted small">
            Different case = case-only rename (handled via two-step on
            case-insensitive disks).
          </p>

          <fieldset className="field-group">
            <legend className="field-label">Collision policy</legend>
            <label className="radio-row">
              <input
                type="radio"
                name="collision"
                checked={collision === "none"}
                onChange={() => setCollision("none")}
              />
              <span>None — abort if target exists</span>
            </label>
            <label className="radio-row">
              <input
                type="radio"
                name="collision"
                checked={collision === "merge"}
                onChange={() => setCollision("merge")}
              />
              <span>Merge (old wins)</span>
            </label>
            <label className="radio-row">
              <input
                type="radio"
                name="collision"
                checked={collision === "overwrite"}
                onChange={() => setCollision("overwrite")}
              />
              <span>Overwrite</span>
            </label>
          </fieldset>

          <label className="check-row">
            <input
              type="checkbox"
              checked={noMove}
              onChange={(e) => setNoMove(e.target.checked)}
            />
            <span>
              <strong>State-only</strong> — update CC state, don't move the
              source directory
            </span>
          </label>

          <fieldset className="danger-zone">
            <legend>
              <AlertTriangle strokeWidth={2.5} /> Danger zone
            </legend>
            <label className="check-row">
              <input
                type="checkbox"
                checked={force}
                onChange={(e) => setForce(e.target.checked)}
              />
              <span>
                <strong>--force</strong> — skip live-session detection. If CC
                is running against this project, its session files can be
                corrupted.
              </span>
            </label>
            <label className="check-row">
              <input
                type="checkbox"
                checked={ignorePending}
                onChange={(e) => setIgnorePending(e.target.checked)}
              />
              <span>
                <strong>--ignore-pending-journals</strong> — run even if a
                prior rename left a journal behind. Resolve pending journals
                first via Repair unless you know why this one is safe.
              </span>
            </label>
          </fieldset>

          <div className="preview-pane" aria-live="polite">
            <div className="field-label">Preview</div>
            {preview.kind === "idle" && (
              <p className="muted small">Enter a new path to preview.</p>
            )}
            {preview.kind === "loading" && (
              <p className="muted small">Computing preview…</p>
            )}
            {preview.kind === "error" && (
              <div className="banner warn">
                <strong>Invalid:</strong>{" "}
                <span className="mono small">{preview.message}</span>
              </div>
            )}
            {preview.kind === "ok" && (
              <ul className="preview-list">
                <li>
                  {preview.plan.would_move_dir ? "Will" : "Won't"} move source
                  directory
                </li>
                <li>
                  CC dir:{" "}
                  <code className="mono">{preview.plan.old_cc_dir}</code> →{" "}
                  <code className="mono">{preview.plan.new_cc_dir}</code>
                </li>
                <li>
                  {preview.plan.session_count} session
                  {preview.plan.session_count === 1 ? "" : "s"},{" "}
                  {preview.plan.estimated_jsonl_files} jsonl file
                  {preview.plan.estimated_jsonl_files === 1 ? "" : "s"} to
                  rewrite
                </li>
                <li>
                  ~/.claude.json:{" "}
                  {preview.plan.would_rewrite_claude_json ? "rewrite" : "skip"}
                </li>
                <li>
                  Auto-memory dir:{" "}
                  {preview.plan.would_move_memory_dir ? "move" : "skip"}
                </li>
                <li>
                  Project-local settings:{" "}
                  {preview.plan.would_rewrite_project_settings
                    ? "rewrite"
                    : "skip"}
                </li>
                {preview.plan.estimated_history_lines > 0 && (
                  <li>
                    History lines potentially updated: ~
                    {preview.plan.estimated_history_lines}
                  </li>
                )}
                {conflict && (
                  <li className="bad">
                    <strong>Conflict:</strong> {conflict}
                    {collision === "none" && (
                      <>
                        {" "}
                        — pick <em>Merge</em> or <em>Overwrite</em>.
                      </>
                    )}
                  </li>
                )}
              </ul>
            )}
          </div>
        </div>

        <div className="modal-actions">
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="primary"
            disabled={submitDisabled}
            onClick={() => onSubmit(args)}
          >
            Rename
          </button>
        </div>
        <p className="muted small submit-disclaimer">
          Preview is approximate. Live-session and pending-journal checks
          happen at apply time.
        </p>
      </div>
    </div>
  );
}

function currentBasename(path: string): string {
  return path.split("/").filter(Boolean).pop() ?? "";
}
