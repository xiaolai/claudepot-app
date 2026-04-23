import { useCallback, useId, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Icon } from "../../components/Icon";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import {
  Modal,
  ModalHeader,
  ModalBody,
  ModalFooter,
} from "../../components/primitives/Modal";
import type {
  AdoptReport,
  DiscardReport,
  OrphanedProject,
} from "../../types";
import { formatSize } from "./format";

type RowState =
  | { kind: "idle" }
  | { kind: "adopting" }
  | { kind: "removing" }
  | { kind: "done"; report: AdoptReport }
  | { kind: "removed"; report: DiscardReport }
  | { kind: "error"; message: string };

/**
 * Orphan adoption modal. One row per orphan; each row carries its own
 * target-cwd input and an Adopt button so the user can rescue orphans
 * into distinct targets in one sitting.
 *
 * Design-principle anchors:
 *   §3 destructive actions state consequence inline — the per-row
 *      status strip reports how many sessions moved and how many
 *      history entries followed.
 *   §5 one signal per surface — success state lives on the row; no
 *      duplicate toast fires while the modal is open.
 */
export function AdoptOrphansModal({
  orphans,
  onClose,
  onCompleted,
}: {
  orphans: OrphanedProject[];
  onClose: () => void;
  /** Called after every user adoption so the section can refresh. */
  onCompleted: () => void;
}) {
  const headingId = useId();

  const initialTargets: Record<string, string> = {};
  orphans.forEach((o) => {
    initialTargets[o.slug] = o.suggestedAdoptionTarget ?? "";
  });
  const [targets, setTargets] = useState<Record<string, string>>(initialTargets);
  const [states, setStates] = useState<Record<string, RowState>>({});
  // Which orphan is pending a Remove confirmation, if any. Per-row state
  // would work too but a single-modal-at-a-time flow is the simpler UX
  // and matches how the rest of the app gates destructive actions.
  const [confirmRemove, setConfirmRemove] = useState<OrphanedProject | null>(
    null,
  );

  const browse = useCallback(async (slug: string) => {
    const picked = await openDialog({
      directory: true,
      multiple: false,
      title: "Choose adoption target directory",
    });
    if (typeof picked === "string") {
      setTargets((t) => ({ ...t, [slug]: picked }));
    }
  }, []);

  const adopt = useCallback(
    async (slug: string) => {
      const target = targets[slug]?.trim();
      if (!target) {
        setStates((s) => ({ ...s, [slug]: { kind: "error", message: "Target required" } }));
        return;
      }
      setStates((s) => ({ ...s, [slug]: { kind: "adopting" } }));
      try {
        const report = await api.sessionAdoptOrphan(slug, target);
        setStates((s) => ({ ...s, [slug]: { kind: "done", report } }));
        onCompleted();
      } catch (e) {
        setStates((s) => ({ ...s, [slug]: { kind: "error", message: String(e) } }));
      }
    },
    [targets, onCompleted],
  );

  const remove = useCallback(
    async (slug: string) => {
      setConfirmRemove(null);
      setStates((s) => ({ ...s, [slug]: { kind: "removing" } }));
      try {
        const report = await api.sessionDiscardOrphan(slug);
        setStates((s) => ({ ...s, [slug]: { kind: "removed", report } }));
        onCompleted();
      } catch (e) {
        setStates((s) => ({ ...s, [slug]: { kind: "error", message: String(e) } }));
      }
    },
    [onCompleted],
  );

  return (
    <Modal open onClose={onClose} width="lg" aria-labelledby={headingId}>
      <ModalHeader
        title="Adopt orphaned projects"
        id={headingId}
        onClose={onClose}
      />
      <ModalBody>
        <p className="muted" style={{ marginTop: 0 }}>
          Each orphan's original cwd no longer exists. <strong>Adopt</strong>{" "}
          to keep the history — choose a live target cwd and every
          session transcript is rewritten so <code>--resume</code> will
          cd into the new target. <strong>Remove</strong> to forget the
          orphan entirely — the slug dir is moved to the Trash and can
          be restored from there if you change your mind.
        </p>

        <ul className="adopt-orphans-list" role="list">
          {orphans.map((o) => {
            const state = states[o.slug] ?? { kind: "idle" };
            const target = targets[o.slug] ?? "";
            // Lock the row once any terminal-or-in-flight action is
            // underway; the only exit after "removed" is closing the
            // modal and re-opening with the refreshed orphan list.
            const disabled =
              state.kind === "adopting" ||
              state.kind === "removing" ||
              state.kind === "done" ||
              state.kind === "removed";
            return (
              <li key={o.slug} className="adopt-orphans-row">
                <div className="adopt-orphans-row-head">
                  <code className="mono selectable">
                    {o.cwdFromTranscript ?? "(unparseable)"}
                  </code>
                  <span className="muted">
                    {o.sessionCount} session{o.sessionCount === 1 ? "" : "s"}
                    {" · "}
                    {formatSize(o.totalSizeBytes)}
                  </span>
                </div>

                <div className="adopt-orphans-row-input">
                  <input
                    type="text"
                    className="path-input pm-focus"
                    placeholder="Target cwd (absolute path)"
                    value={target}
                    onChange={(e) =>
                      setTargets((t) => ({ ...t, [o.slug]: e.target.value }))
                    }
                    disabled={disabled}
                  />
                  <Button
                    variant="ghost"
                    onClick={() => browse(o.slug)}
                    disabled={disabled}
                  >
                    Browse…
                  </Button>
                  <Button
                    variant="solid"
                    onClick={() => adopt(o.slug)}
                    disabled={disabled || !target.trim()}
                  >
                    {state.kind === "adopting" ? "Adopting…" : "Adopt"}
                  </Button>
                  <Button
                    variant="ghost"
                    danger
                    onClick={() => setConfirmRemove(o)}
                    disabled={disabled}
                    title="Move this orphan's slug dir to the Trash"
                  >
                    {state.kind === "removing" ? "Removing…" : "Remove"}
                  </Button>
                </div>

                {state.kind === "done" && (
                  <p className="adopt-orphans-row-status ok">
                    <Icon name="check" size={12} /> Adopted{" "}
                    {state.report.sessionsMoved}/{state.report.sessionsAttempted}{" "}
                    sessions
                    {state.report.sessionsFailed.length > 0 && (
                      <>
                        {", "}
                        {state.report.sessionsFailed.length} failed
                      </>
                    )}
                    .
                  </p>
                )}
                {state.kind === "removed" && (
                  <p className="adopt-orphans-row-status ok">
                    <Icon name="check" size={12} /> Moved{" "}
                    {state.report.sessionsDiscarded} session
                    {state.report.sessionsDiscarded === 1 ? "" : "s"} ·{" "}
                    {formatSize(state.report.totalSizeBytes)} to Trash.
                  </p>
                )}
                {state.kind === "error" && (
                  <p className="adopt-orphans-row-status bad">
                    <Icon name="alert-circle" size={12} /> {state.message}
                  </p>
                )}
              </li>
            );
          })}
        </ul>
      </ModalBody>
      <ModalFooter>
        <Button variant="ghost" onClick={onClose}>
          Close
        </Button>
      </ModalFooter>
      {confirmRemove && (
        <ConfirmDialog
          title="Move orphan to Trash?"
          body={
            <>
              <p style={{ marginTop: 0 }}>
                <code className="mono">
                  {confirmRemove.cwdFromTranscript ?? confirmRemove.slug}
                </code>
              </p>
              <p className="muted" style={{ marginBottom: 0 }}>
                {confirmRemove.sessionCount} session
                {confirmRemove.sessionCount === 1 ? "" : "s"} ·{" "}
                {formatSize(confirmRemove.totalSizeBytes)} will be moved
                to the Trash. You can restore from Trash if you change
                your mind.
              </p>
            </>
          }
          confirmLabel="Move to Trash"
          confirmDanger
          onCancel={() => setConfirmRemove(null)}
          onConfirm={() => remove(confirmRemove.slug)}
        />
      )}
    </Modal>
  );
}
