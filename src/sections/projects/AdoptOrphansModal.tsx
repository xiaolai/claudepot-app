import { useCallback, useId, useState } from "react";
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
import type { AdoptReport, OrphanedProject } from "../../types";
import { formatSize } from "./format";

type RowState =
  | { kind: "idle" }
  | { kind: "adopting" }
  | { kind: "done"; report: AdoptReport }
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

  return (
    <Modal open onClose={onClose} width="lg" aria-labelledby={headingId}>
      <ModalHeader
        title="Adopt orphaned projects"
        id={headingId}
        onClose={onClose}
      />
      <ModalBody>
        <p className="muted" style={{ marginTop: 0 }}>
          Each orphan's original cwd no longer exists. Choose a live
          target cwd to adopt sessions into. Every session transcript
          is rewritten so <code>--resume</code> will cd into the new
          target.
        </p>

        <ul className="adopt-orphans-list" role="list">
          {orphans.map((o) => {
            const state = states[o.slug] ?? { kind: "idle" };
            const target = targets[o.slug] ?? "";
            const disabled = state.kind === "adopting" || state.kind === "done";
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
    </Modal>
  );
}
