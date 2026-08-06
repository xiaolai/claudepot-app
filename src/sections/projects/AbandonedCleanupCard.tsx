import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";
import { Button } from "../../components/primitives/Button";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import {
  Modal,
  ModalBody,
  ModalFooter,
  ModalHeader,
} from "../../components/primitives/Modal";
import { Icon } from "../../components/Icon";
import type { AbandonedCleanupReport } from "../../types";
import { formatSize } from "./format";

/**
 * "Clean recovery artifacts" card — surfaces abandoned rename
 * journals (written by the user hitting "Abandon" in the Repair
 * view) plus the snapshot files they reference, so the user can
 * purge them without dropping to the CLI.
 *
 * Backend contract (`repair_cleanup_abandoned`): only touches files
 * linked to a journal that has an `.abandoned.json` sidecar. Unlike
 * `repair_gc(0, …)`, snapshots from successful ops and live pending
 * journals are left alone — see core test
 * `cleanup_abandoned_leaves_unreferenced_and_non_abandoned_artifacts_alone`.
 *
 * Render policy: the whole card is hidden when there's nothing to
 * clean, per design §Non-negotiables "Render-if-nonzero".
 */
export function AbandonedCleanupCard({
  onCleaned,
}: {
  /** Fired after a successful cleanup so the parent can re-load
   *  the Repair queue (which shares journals-dir state). */
  onCleaned?: () => void;
}) {
  const { t } = useTranslation("projects");
  const [preview, setPreview] = useState<AbandonedCleanupReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [previewOpen, setPreviewOpen] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [cleaning, setCleaning] = useState(false);

  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const refresh = useCallback(async () => {
    try {
      const r = await api.repairPreviewAbandoned();
      if (!mountedRef.current) return;
      setPreview(r);
      setError(null);
    } catch (e) {
      if (!mountedRef.current) return;
      setError(renderError(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const runClean = useCallback(async () => {
    setConfirming(false);
    setCleaning(true);
    try {
      await api.repairCleanupAbandoned();
      onCleaned?.();
      await refresh();
    } catch (e) {
      if (!mountedRef.current) return;
      setError(renderError(e));
    } finally {
      if (mountedRef.current) setCleaning(false);
    }
  }, [refresh, onCleaned]);

  // Render-if-nonzero: hide the section entirely when there are no
  // abandoned artifacts. Surface errors inline only if the preview
  // itself failed (distinct from the empty case).
  if (!preview || preview.entries.length === 0) {
    if (error) {
      return (
        <section className="maintenance-section">
          <div className="maintenance-section-header">
            <Icon name="alert-circle" size={14} />
            <h2>{t("artifacts.heading")}</h2>
          </div>
          <p className="muted maintenance-desc">
            {t("artifacts.listFailed")} <span className="mono">{error}</span>
          </p>
        </section>
      );
    }
    return null;
  }

  const journalCount = preview.entries.length;
  const snapshotCount = preview.entries.reduce(
    (sum, e) => sum + e.referencedSnapshots.length,
    0,
  );
  const totalBytes = preview.entries.reduce((sum, e) => sum + e.bytes, 0);

  // Short summary line. Render-if-nonzero applies to the sub-parts
  // too — snapshot count is only shown when there are any.
  const summary = (() => {
    const parts = [
      t("artifacts.abandonedJournals", { count: journalCount }),
    ];
    if (snapshotCount > 0) {
      parts.push(t("shared.snapshots", { count: snapshotCount }));
    }
    parts.push(formatSize(totalBytes));
    return parts.join(" · ");
  })();

  return (
    <section className="maintenance-section">
      <div className="maintenance-section-header">
        <Icon name="trash-2" size={14} />
        <h2>{t("artifacts.heading")}</h2>
      </div>
      <p className="muted maintenance-desc">
        {t("artifacts.desc")}{" "}
        <span className="mono">{summary}</span>
      </p>
      <div
        style={{
          display: "flex",
          gap: "var(--sp-8)",
          alignItems: "center",
          flexWrap: "wrap",
        }}
      >
        <Button
          variant="ghost"
          onClick={() => setPreviewOpen(true)}
          disabled={cleaning}
        >
          {t("artifacts.preview")}
        </Button>
        <Button
          variant="solid"
          danger
          onClick={() => setConfirming(true)}
          disabled={cleaning}
        >
          {cleaning ? t("artifacts.cleaning") : t("artifacts.clean")}
        </Button>
      </div>

      {previewOpen && (
        <PreviewModal
          report={preview}
          onClose={() => setPreviewOpen(false)}
        />
      )}

      {confirming && (
        <ConfirmDialog
          title={t("artifacts.confirmTitle")}
          body={
            <>
              <p style={{ marginTop: 0 }}>
                {t("artifacts.journalCount", { count: journalCount })}
                {snapshotCount > 0 && (
                  <>
                    {" · "}
                    {t("shared.snapshots", { count: snapshotCount })}
                  </>
                )}{" "}
                · {t("artifacts.willBeRemoved", { size: formatSize(totalBytes) })}
              </p>
              <p className="muted" style={{ marginBottom: 0 }}>
                {t("artifacts.confirmNote")}
              </p>
            </>
          }
          confirmLabel={t("artifacts.delete")}
          confirmDanger
          onCancel={() => setConfirming(false)}
          onConfirm={runClean}
        />
      )}
    </section>
  );
}

function PreviewModal({
  report,
  onClose,
}: {
  report: AbandonedCleanupReport;
  onClose: () => void;
}) {
  const { t } = useTranslation("projects");
  return (
    <Modal open onClose={onClose} width="lg">
      <ModalHeader
        title={t("artifacts.previewTitle", { n: report.entries.length })}
        onClose={onClose}
      />
      <ModalBody>
        <ul className="adopt-orphans-list" role="list">
          {report.entries.map((e) => (
            <li key={e.id} className="adopt-orphans-row">
              <div className="adopt-orphans-row-head">
                <code className="mono selectable">{e.id}</code>
                <span className="muted">{formatSize(e.bytes)}</span>
              </div>
              <div style={{ display: "grid", gap: "var(--sp-4)" }}>
                <div>
                  <span className="muted">{t("artifacts.journalLabel")}</span>{" "}
                  <code className="mono small selectable">{e.journalPath}</code>
                </div>
                <div>
                  <span className="muted">{t("artifacts.sidecarLabel")}</span>{" "}
                  <code className="mono small selectable">{e.sidecarPath}</code>
                </div>
                {e.referencedSnapshots.map((s) => (
                  <div key={s}>
                    <span className="muted">{t("artifacts.snapshotLabel")}</span>{" "}
                    <code className="mono small selectable">{s}</code>
                  </div>
                ))}
              </div>
            </li>
          ))}
        </ul>
      </ModalBody>
      <ModalFooter>
        <Button variant="ghost" onClick={onClose}>
          {t("shared.close")}
        </Button>
      </ModalFooter>
    </Modal>
  );
}
