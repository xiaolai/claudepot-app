import { useCallback, useId, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";
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
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";

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
  const { t } = useTranslation("projects");
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
      title: t("adopt.chooseDirTitle"),
    });
    if (typeof picked === "string") {
      setTargets((t) => ({ ...t, [slug]: picked }));
    }
  }, [t]);

  const adopt = useCallback(
    async (slug: string) => {
      const target = targets[slug]?.trim();
      if (!target) {
        setStates((s) => ({ ...s, [slug]: { kind: "error", message: t("adopt.targetRequired") } }));
        return;
      }
      setStates((s) => ({ ...s, [slug]: { kind: "adopting" } }));
      try {
        const report = await api.sessionAdoptOrphan(slug, target);
        setStates((s) => ({ ...s, [slug]: { kind: "done", report } }));
        onCompleted();
      } catch (e) {
        setStates((s) => ({ ...s, [slug]: { kind: "error", message: renderError(e) } }));
      }
    },
    [targets, onCompleted, t],
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
        setStates((s) => ({ ...s, [slug]: { kind: "error", message: renderError(e) } }));
      }
    },
    [onCompleted],
  );

  return (
    <Modal open onClose={onClose} width="lg" aria-labelledby={headingId}>
      <ModalHeader
        title={t("adopt.title")}
        id={headingId}
        onClose={onClose}
      />
      <ModalBody>
        <p className="muted" style={{ marginTop: 0 }}>
          <Trans
            ns="projects"
            i18nKey="adopt.intro"
            components={{
              adopt: <strong />,
              resume: <code>--resume</code>,
              remove: <strong />,
            }}
          />
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
                    {o.cwdFromTranscript ?? t("adopt.unparseable")}
                  </code>
                  <span className="muted">
                    {t("shared.sessions", { count: o.sessionCount })}
                    {" · "}
                    {formatSize(o.totalSizeBytes)}
                  </span>
                </div>

                <div className="adopt-orphans-row-input">
                  <input
                    type="text"
                    className="path-input pm-focus"
                    placeholder={t("shared.targetCwd")}
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
                    {t("shared.browse")}
                  </Button>
                  <Button
                    variant="solid"
                    onClick={() => adopt(o.slug)}
                    disabled={disabled || !target.trim()}
                  >
                    {state.kind === "adopting" ? t("adopt.adopting") : t("adopt.adopt")}
                  </Button>
                  <Button
                    variant="ghost"
                    danger
                    onClick={() => setConfirmRemove(o)}
                    disabled={disabled}
                    title={t("adopt.removeTitle")}
                  >
                    {state.kind === "removing" ? t("adopt.removing") : t("adopt.remove")}
                  </Button>
                </div>

                {state.kind === "done" && (
                  <p className="adopt-orphans-row-status ok">
                    <Glyph g={NF.check} style={{ fontSize: 12 }} />{" "}
                    {t("adopt.adoptedStatus", {
                      moved: state.report.sessionsMoved,
                      attempted: state.report.sessionsAttempted,
                    })}
                    {state.report.sessionsFailed.length > 0 &&
                      t("adopt.failedSuffix", {
                        n: state.report.sessionsFailed.length,
                      })}
                    .
                  </p>
                )}
                {state.kind === "removed" && (
                  <p className="adopt-orphans-row-status ok">
                    <Glyph g={NF.check} style={{ fontSize: 12 }} />{" "}
                    {t("adopt.removedStatus", {
                      count: state.report.sessionsDiscarded,
                      size: formatSize(state.report.totalSizeBytes),
                    })}
                  </p>
                )}
                {state.kind === "error" && (
                  <p className="adopt-orphans-row-status bad">
                    <Glyph g={NF.alertCircle} style={{ fontSize: 12 }} /> {state.message}
                  </p>
                )}
              </li>
            );
          })}
        </ul>
      </ModalBody>
      <ModalFooter>
        <Button variant="ghost" onClick={onClose}>
          {t("shared.close")}
        </Button>
      </ModalFooter>
      {confirmRemove && (
        <ConfirmDialog
          title={t("adopt.confirmTitle")}
          body={
            <>
              <p style={{ marginTop: 0 }}>
                <code className="mono">
                  {confirmRemove.cwdFromTranscript ?? confirmRemove.slug}
                </code>
              </p>
              <p className="muted" style={{ marginBottom: 0 }}>
                {t("adopt.confirmBody", {
                  count: confirmRemove.sessionCount,
                  size: formatSize(confirmRemove.totalSizeBytes),
                })}
              </p>
            </>
          }
          confirmLabel={t("adopt.confirmLabel")}
          confirmDanger
          onCancel={() => setConfirmRemove(null)}
          onConfirm={() => remove(confirmRemove.slug)}
        />
      )}
    </Modal>
  );
}
