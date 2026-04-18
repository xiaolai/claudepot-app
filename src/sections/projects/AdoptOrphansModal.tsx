import { useCallback, useEffect, useRef, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Icon } from "../../components/Icon";
import { api } from "../../api";
import { useFocusTrap } from "../../hooks/useFocusTrap";
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
  const headingId = useRef(`adopt-heading-${Math.random().toString(36).slice(2, 9)}`);
  const trapRef = useFocusTrap<HTMLDivElement>();

  // Per-orphan row state + per-row target cwd input.
  const initialTargets: Record<string, string> = {};
  orphans.forEach((o) => {
    initialTargets[o.slug] = o.suggestedAdoptionTarget ?? "";
  });
  const [targets, setTargets] = useState<Record<string, string>>(initialTargets);
  const [states, setStates] = useState<Record<string, RowState>>({});

  useEffect(() => {
    const onEsc = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onEsc);
    return () => window.removeEventListener("keydown", onEsc);
  }, [onClose]);

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
    <div
      className="modal-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={trapRef}
        className="modal adopt-orphans-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby={headingId.current}
      >
        <h2 id={headingId.current}>Adopt orphaned projects</h2>
        <p className="muted">
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
                    className="path-input"
                    placeholder="Target cwd (absolute path)"
                    value={target}
                    onChange={(e) =>
                      setTargets((t) => ({ ...t, [o.slug]: e.target.value }))
                    }
                    disabled={disabled}
                  />
                  <button
                    className="btn"
                    onClick={() => browse(o.slug)}
                    disabled={disabled}
                  >
                    Browse…
                  </button>
                  <button
                    className="btn primary"
                    onClick={() => adopt(o.slug)}
                    disabled={disabled || !target.trim()}
                  >
                    {state.kind === "adopting" ? "Adopting…" : "Adopt"}
                  </button>
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

        <div className="modal-actions">
          <button className="btn" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

