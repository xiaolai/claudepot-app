import { useCallback, useId, useMemo, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";
import { Button } from "../../components/primitives/Button";
import { Glyph } from "../../components/primitives/Glyph";
import {
  Modal,
  ModalHeader,
  ModalBody,
  ModalFooter,
} from "../../components/primitives/Modal";
import { Disclosure, OptionRow } from "../../components/primitives/modalParts";
import { useOperations } from "../../hooks/useOperations";
import { NF } from "../../icons";
import { basename } from "../../lib/paths";
import type { MoveSessionReport, ProjectInfo } from "../../types";
import { classifyProject } from "./projectStatus";
import {
  MoveTargetPicker,
  type ResolvedMoveTarget,
} from "./MoveTargetPicker";
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
 * Target picking lives in `MoveTargetPicker` — a filterable list of live
 * projects plus a path escape hatch that can name a folder which does
 * not exist yet. This component owns only the move itself.
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
  const { t } = useTranslation("projects");
  const headingId = useId();
  const { open: openOpModal } = useOperations();

  // Listed targets: only "alive" projects — offering an orphan /
  // unreachable / empty one would either fail the backend or rewrite cwd
  // to a path that doesn't exist. A folder outside this set is still
  // reachable, by typing or browsing to it in the picker.
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
  // `null` until the picker has a target it will vouch for — a project
  // row, or a path it has probed. A blocked state (still checking, not
  // absolute, points at a file) reads as "no target" here, so this
  // component can't submit something the picker rejected.
  const [target, setTarget] = useState<ResolvedMoveTarget | null>(null);
  const [forceLive, setForceLive] = useState(false);
  const [forceConflict, setForceConflict] = useState(false);
  const [cleanupSource, setCleanupSource] = useState(false);
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });

  const canSubmit =
    phase.kind === "idle" && target !== null && target.path !== fromCwd;
  const starting = phase.kind === "starting";

  // Escape is suppressed while the submit is in flight — Modal wires
  // its own Escape handler, so we gate the onClose callback.
  const handleClose = () => {
    if (!starting) onClose();
  };

  // Stable identity: the picker reports upward from an effect, and an
  // inline lambda here would re-run it on every render of this modal.
  const handleResolve = useCallback((next: ResolvedMoveTarget | null) => {
    setTarget(next);
  }, []);

  async function submit() {
    if (!canSubmit || !target) return;
    setPhase({ kind: "starting" });
    try {
      const opId = await api.sessionMoveStart({
        sessionId,
        fromCwd,
        toCwd: target.path,
        forceLive,
        forceConflict,
        cleanupSource,
        createTargetDir: target.createDir,
      });
      const shortFromBase = basename(fromCwd);
      const shortToBase = basename(target.path);
      openOpModal({
        opId,
        title: t("move.progressTitle", {
          sid: sessionId.slice(0, 8),
          to: shortToBase,
        }),
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
      setPhase({ kind: "error", message: renderError(e) });
    }
  }

  const shortSid = sessionId.slice(0, 8);
  const shortFrom = basename(fromCwd);
  const shortTo = target ? basename(target.path) : "";

  return (
    <Modal open onClose={handleClose} width="lg" aria-labelledby={headingId}>
      <ModalHeader
        glyph={NF.arrowR}
        title={t("move.title")}
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
          <span className="mono-cap">{t("move.sessionLabel")}</span>
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
          <Trans
            ns="projects"
            i18nKey="move.explain"
            components={{
              from: (
                <strong className="mono" style={{ color: "var(--fg)" }}>
                  {shortFrom}
                </strong>
              ),
              cwd: (
                <code className="mono" style={{ fontSize: "var(--fs-xs)" }}>
                  cwd
                </code>
              ),
              resume: (
                <code className="mono" style={{ fontSize: "var(--fs-xs)" }}>
                  --resume
                </code>
              ),
            }}
          />
        </p>

        <MoveTargetPicker
          projects={options}
          disabled={starting}
          onResolve={handleResolve}
        />

        <Disclosure label={t("move.advanced")}>
          <OptionRow
            type="checkbox"
            checked={forceLive}
            onChange={(e) => setForceLive(e.target.checked)}
            disabled={starting}
          >
            <strong style={{ fontWeight: 600 }}>
              {t("move.forceLive")}
            </strong>
            <span style={{ color: "var(--fg-faint)" }}>
              {" "}
              {t("move.forceLiveDesc")}
            </span>
          </OptionRow>
          <OptionRow
            type="checkbox"
            checked={forceConflict}
            onChange={(e) => setForceConflict(e.target.checked)}
            disabled={starting}
          >
            <strong style={{ fontWeight: 600 }}>
              <Trans
                ns="projects"
                i18nKey="move.forceConflict"
                components={{
                  pat: (
                    <code className="mono" style={{ fontSize: "var(--fs-xs)" }}>
                      .sync-conflict-*
                    </code>
                  ),
                }}
              />
            </strong>
            <span style={{ color: "var(--fg-faint)" }}>
              {" "}
              {t("move.forceConflictDesc")}
            </span>
          </OptionRow>
          <OptionRow
            type="checkbox"
            checked={cleanupSource}
            onChange={(e) => setCleanupSource(e.target.checked)}
            disabled={starting}
          >
            <strong style={{ fontWeight: 600 }}>
              {t("move.cleanupSource")}
            </strong>
            <span style={{ color: "var(--fg-faint)" }}>
              {" "}
              {t("move.cleanupSourceDesc")}
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
              border: "var(--bw-hair) solid var(--danger)",
              background: "var(--bad-weak)",
              color: "var(--danger)",
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
          {t("shared.cancel")}
        </Button>
        {/* No autoFocus: the picker's filter field claims initial focus,
            because choosing a target is the first thing this modal is
            for. The label names the folder-creating variant explicitly —
            a move that quietly `mkdir`s is a move the user didn't
            authorize. */}
        <Button variant="solid" onClick={submit} disabled={!canSubmit}>
          {starting
            ? t("move.starting")
            : target?.createDir
              ? t("move.createAndMoveTo", { target: shortTo })
              : t("move.moveTo", { target: shortTo || "…" })}
        </Button>
      </ModalFooter>
    </Modal>
  );
}
