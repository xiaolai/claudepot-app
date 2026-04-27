import { useId, useMemo, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { Glyph } from "../../components/primitives/Glyph";
import {
  Modal,
  ModalHeader,
  ModalBody,
  ModalFooter,
} from "../../components/primitives/Modal";
import {
  Disclosure,
  FieldBlock,
  OptionRow,
} from "../../components/primitives/modalParts";
import { useOperations } from "../../hooks/useOperations";
import { NF } from "../../icons";
import type { MoveSessionReport, ProjectInfo } from "../../types";
import { classifyProject } from "./projectStatus";
import {
  SESSION_MOVE_PHASES,
  renderSessionMoveResult,
} from "./sessionMoveProgress";

type Phase =
  | { kind: "idle" }
  | { kind: "starting" }
  | { kind: "error"; message: string };

/**
 * Modal fired from a session-row context menu. Moves one CC session
 * from its current project's cwd to a target cwd.
 *
 * Submit hands off to `api.sessionMoveStart`, which returns an op_id.
 * The shell-level `OperationProgressModal` takes over from there —
 * S1..S5 phase rows render live progress, and the user can close the
 * progress modal to background the op without cancelling it.
 *
 * Target picker: dropdown of existing Claudepot-tracked projects plus
 * an "Other…" escape hatch that opens a native directory picker.
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
  onCompleted: (report: MoveSessionReport | null) => void;
}) {
  const headingId = useId();
  const selectId = useId();
  const customCwdId = useId();
  const { open: openOpModal } = useOperations();

  // Dropdown options: only "alive" projects — picking an orphan /
  // unreachable / empty target would either fail the backend or
  // rewrite cwd to a path that doesn't exist. "Other…" is always
  // available as the free-form escape hatch.
  //
  // Sort: most-recently-touched first so the default selection is
  // the one the user almost certainly wants (B1, B11).
  const options = useMemo(
    () => {
      // Two distinct slugs can unsanitize to the same `original_path`
      // (the round-trip is lossy — see .claude/rules/paths.md). The
      // target of a move is the cwd path itself, so duplicate paths
      // collapse to one option; pick the most-recently-touched slug as
      // the representative so sort below stays stable.
      const alive = projects
        .filter(
          (p) =>
            p.original_path !== fromCwd && classifyProject(p) === "alive",
        )
        .sort(
          (a, b) => (b.last_modified_ms ?? 0) - (a.last_modified_ms ?? 0),
        );
      const seen = new Set<string>();
      return alive.filter((p) => {
        if (seen.has(p.original_path)) return false;
        seen.add(p.original_path);
        return true;
      });
    },
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
  const starting = phase.kind === "starting";

  // Escape is suppressed while the submit is in flight — Modal wires
  // its own Escape handler, so we gate the onClose callback.
  const handleClose = () => {
    if (!starting) onClose();
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
    setPhase({ kind: "starting" });
    try {
      const opId = await api.sessionMoveStart({
        sessionId,
        fromCwd,
        toCwd: target,
        forceLive,
        forceConflict,
        cleanupSource,
      });
      const shortFromBase = basename(fromCwd) ?? fromCwd;
      const shortToBase = basename(target) ?? target;
      openOpModal({
        opId,
        title: `Moving session ${sessionId.slice(0, 8)} → ${shortToBase}`,
        phases: SESSION_MOVE_PHASES,
        fetchStatus: api.sessionMoveStatus,
        renderResult: renderSessionMoveResult,
        onComplete: () => {
          // The shell modal carries the success summary; this caller
          // only needs to know the op terminated so it can refresh.
          onCompleted(null);
        },
        onError: () => {
          // Same idea — the shell modal renders the error; we just
          // notify the parent so it can refresh / clear stale state.
          onCompleted(null);
        },
      });
      // Hand off the user-visible surface to the shell modal.
      onClose();
      // Reference the unused-on-success local so it's clear the close
      // path doesn't depend on it.
      void shortFromBase;
    } catch (e) {
      setPhase({ kind: "error", message: String(e) });
    }
  }

  const shortSid = sessionId.slice(0, 8);
  const shortFrom = basename(fromCwd) ?? fromCwd;
  const shortTo = target ? (basename(target) ?? target) : "";

  return (
    <Modal open onClose={handleClose} width="lg" aria-labelledby={headingId}>
      <ModalHeader
        glyph={NF.arrowR}
        title="Move session"
        id={headingId}
        onClose={handleClose}
      />
      <ModalBody
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-14)",
        }}
      >
        {/* Session identity strip — demoted below the title so the
            8-char prefix isn't the loudest text on the screen. */}
        <div
          style={{
            display: "flex",
            alignItems: "baseline",
            gap: "var(--sp-8)",
            color: "var(--fg-faint)",
            fontSize: "var(--fs-2xs)",
          }}
        >
          <span className="mono-cap">session</span>
          <span className="mono" title={sessionId}>
            {shortSid}
          </span>
        </div>

        <p
          style={{
            margin: 0,
            fontSize: "var(--fs-sm)",
            lineHeight: "var(--lh-body)",
            color: "var(--fg-muted)",
          }}
        >
          From <strong className="mono" style={{ color: "var(--fg)" }}>{shortFrom}</strong>{" "}
          to the target you pick. Every transcript line's{" "}
          <code className="mono" style={{ fontSize: "var(--fs-xs)" }}>cwd</code>{" "}
          is rewritten so{" "}
          <code className="mono" style={{ fontSize: "var(--fs-xs)" }}>--resume</code>{" "}
          opens the new project. History entries keyed by this sessionId
          follow; pre-sessionId entries stay behind.
        </p>

        <FieldBlock label="Target project" htmlFor={selectId}>
          <select
            id={selectId}
            value={selection}
            onChange={(e) => setSelection(e.target.value)}
            disabled={starting}
            className="mono pm-focus"
            style={{
              width: "100%",
              height: "var(--sp-28)",
              padding: "0 var(--sp-8)",
              border: "var(--bw-hair) solid var(--line)",
              borderRadius: "var(--r-2)",
              background: "var(--bg)",
              color: "var(--fg)",
              fontSize: "var(--fs-sm)",
              cursor: starting ? "not-allowed" : "pointer",
            }}
          >
            {options.length === 0 && (
              <option value="__other__" disabled>
                No live project targets — pick Other…
              </option>
            )}
            {options.map((p) => {
              const base = basename(p.original_path) ?? p.original_path;
              return (
                <option key={p.sanitized_name} value={p.original_path}>
                  {base} — {p.original_path}
                </option>
              );
            })}
            <option value="__other__">Other…</option>
          </select>
        </FieldBlock>

        {selection === "__other__" && (
          <FieldBlock label="Custom path" htmlFor={customCwdId}>
            <div
              style={{
                display: "flex",
                gap: "var(--sp-6)",
                alignItems: "stretch",
              }}
            >
              <input
                id={customCwdId}
                type="text"
                className="mono pm-focus"
                placeholder="Target cwd (absolute path)"
                value={customCwd}
                onChange={(e) => setCustomCwd(e.target.value)}
                disabled={starting}
                style={{
                  flex: 1,
                  padding: "var(--sp-6) var(--sp-10)",
                  fontSize: "var(--fs-sm)",
                  color: "var(--fg)",
                  background: "var(--bg)",
                  border: "var(--bw-hair) solid var(--line)",
                  borderRadius: "var(--r-2)",
                  outline: "none",
                }}
              />
              <Button variant="ghost" onClick={browse} disabled={starting}>
                Browse…
              </Button>
            </div>
          </FieldBlock>
        )}

        <Disclosure label="Advanced">
          <OptionRow
            type="checkbox"
            checked={forceLive}
            onChange={(e) => setForceLive(e.target.checked)}
            disabled={starting}
          >
            <strong style={{ fontWeight: 600 }}>
              Force past the live-session mtime guard
            </strong>
            <span style={{ color: "var(--fg-faint)" }}>
              {" "}
              — use only if CC isn't writing to this session.
            </span>
          </OptionRow>
          <OptionRow
            type="checkbox"
            checked={forceConflict}
            onChange={(e) => setForceConflict(e.target.checked)}
            disabled={starting}
          >
            <strong style={{ fontWeight: 600 }}>
              Force past Syncthing{" "}
              <code className="mono" style={{ fontSize: "var(--fs-xs)" }}>
                .sync-conflict-*
              </code>
            </strong>
            <span style={{ color: "var(--fg-faint)" }}>
              {" "}
              — will silently orphan the conflict copy.
            </span>
          </OptionRow>
          <OptionRow
            type="checkbox"
            checked={cleanupSource}
            onChange={(e) => setCleanupSource(e.target.checked)}
            disabled={starting}
          >
            <strong style={{ fontWeight: 600 }}>
              Remove source project dir if it's empty after the move
            </strong>
            <span style={{ color: "var(--fg-faint)" }}>
              {" "}
              — tidy up the husk when this was the last session here.
            </span>
          </OptionRow>
        </Disclosure>

        {phase.kind === "error" && (
          <div
            role="alert"
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-6)",
              margin: 0,
              padding: "var(--sp-8) var(--sp-10)",
              border: "var(--bw-hair) solid var(--bad)",
              background: "var(--bad-weak)",
              color: "var(--bad)",
              borderRadius: "var(--r-2)",
              fontSize: "var(--fs-xs)",
            }}
          >
            <Glyph g={NF.warn} style={{ fontSize: "var(--fs-xs)" }} />
            <span style={{ minWidth: 0, flex: 1, wordBreak: "break-word" }}>
              {phase.message}
            </span>
          </div>
        )}
      </ModalBody>
      <ModalFooter>
        <Button variant="ghost" onClick={handleClose} disabled={starting}>
          Cancel
        </Button>
        <Button
          variant="solid"
          onClick={submit}
          disabled={!canSubmit}
          autoFocus
        >
          {starting ? "Starting…" : `Move to ${shortTo || "…"}`}
        </Button>
      </ModalFooter>
    </Modal>
  );
}

function basename(p: string): string | null {
  const parts = p.split("/").filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : null;
}
