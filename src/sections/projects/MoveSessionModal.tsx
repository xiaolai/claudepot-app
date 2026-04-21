import { useId, useMemo, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Icon } from "../../components/Icon";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import {
  Modal,
  ModalHeader,
  ModalBody,
  ModalFooter,
} from "../../components/primitives/Modal";
import type { MoveSessionReport, ProjectInfo } from "../../types";
import { classifyProject } from "./projectStatus";

type Phase =
  | { kind: "idle" }
  | { kind: "moving" }
  | { kind: "done"; report: MoveSessionReport }
  | { kind: "error"; message: string };

/**
 * Modal fired from a session-row context menu. Moves one CC session
 * from its current project's cwd to a target cwd.
 *
 * Target picker: dropdown of existing Claudepot-tracked projects plus
 * an "Other…" escape hatch that opens a native directory picker.
 *
 * Design-principle anchors:
 *   §3 destructive actions state consequence inline — the confirmation
 *      button carries the verb and the numbers; the hint below spells
 *      out what will follow ("rewrites cwd on N turns, K history
 *      entries follow, M pre-sessionId entries stay behind").
 *   §5 one signal per surface — success state is inline here (no toast);
 *      the caller refreshes when we call onCompleted().
 *   Feedback-ladder: destructive but reversible-via-workflow → modal,
 *      not banner.
 */
export function MoveSessionModal({
  sessionId,
  fromCwd,
  projects,
  onClose,
  onCompleted,
}: {
  sessionId: string;
  fromCwd: string;
  /** Live list of projects (for the target-picker dropdown). */
  projects: ProjectInfo[];
  onClose: () => void;
  /** Called after a successful move so the caller can refresh. */
  onCompleted: (report: MoveSessionReport) => void;
}) {
  const headingId = useId();

  // Dropdown options: only "alive" projects — picking an orphan /
  // unreachable / empty target would either fail the backend or
  // rewrite cwd to a path that doesn't exist. "Other…" is always
  // available as the free-form escape hatch.
  //
  // Sort: most-recently-touched first so the default selection is
  // the one the user almost certainly wants (B1, B11).
  const options = useMemo(
    () =>
      projects
        .filter(
          (p) =>
            p.original_path !== fromCwd && classifyProject(p) === "alive",
        )
        .sort(
          (a, b) => (b.last_modified_ms ?? 0) - (a.last_modified_ms ?? 0),
        ),
    [projects, fromCwd],
  );
  const [selection, setSelection] = useState<string>(
    options[0]?.original_path ?? "__other__",
  );
  const [customCwd, setCustomCwd] = useState("");
  const [forceLive, setForceLive] = useState(false);
  const [forceConflict, setForceConflict] = useState(false);
  const [cleanupSource, setCleanupSource] = useState(false);
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });

  const target = selection === "__other__" ? customCwd.trim() : selection;
  const canSubmit =
    phase.kind === "idle" && target !== "" && target !== fromCwd;

  // Escape is suppressed while a move is in flight — Modal wires
  // its own Escape handler, so we gate the onClose callback.
  const handleClose = () => {
    if (phase.kind !== "moving") onClose();
  };

  async function browse() {
    const picked = await openDialog({
      directory: true,
      multiple: false,
      title: "Choose target project directory",
    });
    if (typeof picked === "string") {
      setSelection("__other__");
      setCustomCwd(picked);
    }
  }

  async function submit() {
    if (!canSubmit) return;
    setPhase({ kind: "moving" });
    try {
      const report = await api.sessionMove({
        sessionId,
        fromCwd,
        toCwd: target,
        forceLive,
        forceConflict,
        cleanupSource,
      });
      setPhase({ kind: "done", report });
      onCompleted(report);
    } catch (e) {
      setPhase({ kind: "error", message: String(e) });
    }
  }

  const shortSid = sessionId.slice(0, 8);
  const shortFrom = basename(fromCwd) ?? fromCwd;
  const shortTo = target ? (basename(target) ?? target) : "";

  return (
    <Modal open onClose={handleClose} aria-labelledby={headingId}>
      <ModalHeader
        title={`Move session ${shortSid}`}
        id={headingId}
        onClose={handleClose}
      />
      <ModalBody>
        <p className="muted" style={{ marginTop: 0 }}>
          From <strong className="mono">{shortFrom}</strong> to the target
          you pick. Every transcript line's <code>cwd</code> is rewritten
          so <code>--resume</code> will cd into the new project.
          History entries keyed by this sessionId follow; pre-sessionId
          entries stay behind.
        </p>

        {phase.kind !== "done" ? (
          <>
            <div className="move-session-form">
              <label className="move-session-label">
                Target project
                <select
                  value={selection}
                  onChange={(e) => setSelection(e.target.value)}
                  disabled={phase.kind === "moving"}
                >
                  {options.length === 0 && (
                    <option value="__other__" disabled>
                      No live project targets — pick Other…
                    </option>
                  )}
                  {options.map((p) => {
                    const base = basename(p.original_path) ?? p.original_path;
                    return (
                      <option key={p.original_path} value={p.original_path}>
                        {base} — {p.original_path}
                      </option>
                    );
                  })}
                  <option value="__other__">Other…</option>
                </select>
              </label>

              {selection === "__other__" && (
                <div className="adopt-orphans-row-input">
                  <input
                    type="text"
                    className="path-input pm-focus"
                    placeholder="Target cwd (absolute path)"
                    value={customCwd}
                    onChange={(e) => setCustomCwd(e.target.value)}
                    disabled={phase.kind === "moving"}
                  />
                  <Button
                    variant="ghost"
                    onClick={browse}
                    disabled={phase.kind === "moving"}
                  >
                    Browse…
                  </Button>
                </div>
              )}

              <details className="move-session-advanced">
                <summary>Advanced</summary>
                <label className="move-session-check">
                  <input
                    type="checkbox"
                    checked={forceLive}
                    onChange={(e) => setForceLive(e.target.checked)}
                    disabled={phase.kind === "moving"}
                  />
                  Force past the live-session mtime guard
                  <span className="muted">
                    (use only if CC isn't writing to this session)
                  </span>
                </label>
                <label className="move-session-check">
                  <input
                    type="checkbox"
                    checked={forceConflict}
                    onChange={(e) => setForceConflict(e.target.checked)}
                    disabled={phase.kind === "moving"}
                  />
                  Force past Syncthing <code>.sync-conflict-*</code>
                  <span className="muted">
                    (will silently orphan the conflict copy)
                  </span>
                </label>
                <label className="move-session-check">
                  <input
                    type="checkbox"
                    checked={cleanupSource}
                    onChange={(e) => setCleanupSource(e.target.checked)}
                    disabled={phase.kind === "moving"}
                  />
                  Remove source project dir if it's empty after the move
                  <span className="muted">
                    (was the last session here? tidy up the husk)
                  </span>
                </label>
              </details>
            </div>

            {phase.kind === "error" && (
              <p className="move-session-error" role="alert">
                <Icon name="alert-circle" size={12} /> {phase.message}
              </p>
            )}
          </>
        ) : (
          <div className="move-session-done">
            <p className="move-session-done-line">
              <Icon name="check" size={14} /> Moved.
            </p>
            <dl className="detail-grid">
              <dt>Transcript lines rewritten</dt>
              <dd>{phase.report.jsonlLinesRewritten}</dd>
              {phase.report.subagentFilesMoved > 0 && (
                <>
                  <dt>Subagent files moved</dt>
                  <dd>{phase.report.subagentFilesMoved}</dd>
                </>
              )}
              {phase.report.remoteAgentFilesMoved > 0 && (
                <>
                  <dt>Remote-agent files moved</dt>
                  <dd>{phase.report.remoteAgentFilesMoved}</dd>
                </>
              )}
              <dt>History entries followed</dt>
              <dd>
                {phase.report.historyEntriesMoved}
                {phase.report.historyEntriesUnmapped > 0 && (
                  <span className="muted">
                    {" · "}
                    {phase.report.historyEntriesUnmapped} stayed (pre-sessionId)
                  </span>
                )}
              </dd>
              {phase.report.claudeJsonPointersCleared > 0 && (
                <>
                  <dt>
                    <code className="mono">.claude.json</code> pointers cleared
                  </dt>
                  <dd>{phase.report.claudeJsonPointersCleared}</dd>
                </>
              )}
              {phase.report.sourceDirRemoved && (
                <>
                  <dt>Source project dir</dt>
                  <dd>removed (was empty)</dd>
                </>
              )}
            </dl>
          </div>
        )}
      </ModalBody>
      <ModalFooter>
        {phase.kind !== "done" ? (
          <>
            <Button
              variant="ghost"
              onClick={handleClose}
              disabled={phase.kind === "moving"}
            >
              Cancel
            </Button>
            <Button
              variant="solid"
              onClick={submit}
              disabled={!canSubmit}
              autoFocus
            >
              {phase.kind === "moving"
                ? "Moving…"
                : `Move to ${shortTo || "…"}`}
            </Button>
          </>
        ) : (
          <Button variant="solid" onClick={onClose} autoFocus>
            Close
          </Button>
        )}
      </ModalFooter>
    </Modal>
  );
}

function basename(p: string): string | null {
  const parts = p.split("/").filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : null;
}
