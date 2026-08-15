import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";
import { useOperations } from "../../hooks/useOperations";
import { useAppState } from "../../providers/AppStateProvider";
import type { JournalEntry } from "../../types";
import { RepairEntry } from "./RepairEntry";
import { RepairConfirmDialogs, type PendingAction } from "./RepairConfirmDialogs";
import { ConfirmDangerousAction } from "../../components/ConfirmDangerousAction";
import { SkeletonRows } from "../../components/primitives/Skeleton";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";

export function RepairView({
  onBack,
  onOpTerminated,
  embedded,
}: {
  onBack: () => void;
  onOpTerminated?: () => void;
  embedded?: boolean;
}) {
  const { t } = useTranslation("projects");
  const [entries, setEntries] = useState<JournalEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingAction | null>(null);
  const [breakLockTarget, setBreakLockTarget] = useState<JournalEntry | null>(
    null,
  );
  const { pushToast } = useAppState();
  const { open: openOpModal } = useOperations();

  const refresh = useCallback(() => {
    setLoading(true);
    api.repairList()
      .then((es) => {
        setEntries([...es].sort((a, b) => b.started_unix_secs - a.started_unix_secs));
        setLoading(false);
        setError(null);
      })
      .catch((e) => { setError(renderError(e)); setLoading(false); });
  }, []);

  useEffect(() => { refresh(); }, [refresh]);

  // Audit M18: distinct terminal handlers so a failed repair doesn't
  // show a "Done." toast. Previously both onComplete and onError
  // pointed at the same afterTerminal which always set "Done." —
  // indistinguishable from success at the page level.
  const kindLabel = (kind: "Resume" | "Rollback") =>
    kind === "Resume" ? t("repair.kindResume") : t("repair.kindRollback");
  const afterComplete = (kind: "Resume" | "Rollback", id: string) => {
    pushToast("info", t("repair.completeToast", { kind: kindLabel(kind), id }));
    refresh();
    onOpTerminated?.();
  };
  const afterError = (kind: "Resume" | "Rollback", id: string, detail: string | null) => {
    pushToast(
      "error",
      renderError(detail ?? id, t("repair.failedScope", { kind: kindLabel(kind) })),
    );
    refresh();
    onOpTerminated?.();
  };

  const runResume = async (entry: JournalEntry) => {
    setPending(null);
    try {
      const opId = await api.repairResumeStart(entry.id);
      openOpModal({
        opId,
        title: t("repair.resumingTitle", { id: entry.id }),
        onComplete: () => afterComplete("Resume", entry.id),
        onError: (detail) => afterError("Resume", entry.id, detail),
      });
    } catch (e) { pushToast("error", renderError(e, t("repair.resumeFailedScope"))); }
  };

  const runRollback = async (entry: JournalEntry) => {
    setPending(null);
    try {
      const opId = await api.repairRollbackStart(entry.id);
      openOpModal({
        opId,
        title: t("repair.rollingBackTitle", { id: entry.id }),
        onComplete: () => afterComplete("Rollback", entry.id),
        onError: (detail) => afterError("Rollback", entry.id, detail),
      });
    } catch (e) { pushToast("error", renderError(e, t("repair.rollbackFailedScope"))); }
  };

  const runAbandon = async (entry: JournalEntry) => {
    setPending(null);
    try {
      await api.repairAbandon(entry.id);
      pushToast("info", t("repair.abandonedToast", { id: entry.id }));
      refresh();
      onOpTerminated?.();
    } catch (e) { pushToast("error", renderError(e, t("repair.abandonFailedScope"))); }
  };

  const runBreakLock = async (entry: JournalEntry) => {
    setBreakLockTarget(null);
    try {
      const outcome = await api.repairBreakLock(entry.old_path);
      pushToast(
        "info",
        t("repair.lockBrokenToast", {
          pid: outcome.prior_pid,
          host: outcome.prior_hostname,
        }),
      );
      refresh();
      onOpTerminated?.();
    } catch (e) {
      pushToast("error", renderError(e, t("repair.breakLockFailedScope")));
    }
  };

  const Wrapper = embedded ? "div" : "main";

  return (
    <Wrapper className={embedded ? "repair-view-embedded" : "content repair-view"}>
      {!embedded && (
        <header className="repair-header">
          <button type="button" className="icon-btn" onClick={onBack}
            aria-label={t("repair.backTitle")} title={t("repair.backTitle")}>
            <Glyph g={NF.arrowL} style={{ fontSize: 14 }} />
          </button>
          <h2><Glyph g={NF.tools} style={{ fontSize: 14 }} /> {t("repair.heading")}</h2>
        </header>
      )}

      {loading && entries.length === 0 && (
        <SkeletonRows rows={1} />
      )}
      {error && (
        <div className="banner warn" role="alert">
          <div><strong>{t("repair.loadFailed")}</strong> <span className="mono">{error}</span></div>
        </div>
      )}
      {!loading && !error && entries.length === 0 && (
        <div className="empty">
          <Glyph g={NF.tools} style={{ fontSize: 32 }} />
          <h2>{t("repair.allClear")}</h2>
          <p className="muted">{t("repair.noPending")}</p>
        </div>
      )}

      {entries.length > 0 && (
        <ul className="repair-list">
          {entries.map((e) => (
            <RepairEntry key={e.id} entry={e}
              onResume={() => setPending({ kind: "resume", entry: e })}
              onRollback={() => setPending({ kind: "rollback", entry: e })}
              onAbandon={() => setPending({ kind: "abandon", entry: e })}
              onBreakLock={() => setBreakLockTarget(e)} />
          ))}
        </ul>
      )}

      <RepairConfirmDialogs pending={pending} onCancel={() => setPending(null)}
        onResume={runResume} onRollback={runRollback} onAbandon={runAbandon} />

      {breakLockTarget && (
        <ConfirmDangerousAction
          title={t("repair.breakLockTitle")}
          confirmLabel={t("repair.breakLock")}
          consequences={
            <>
              <p>
                {t("repair.breakLockBody")}
              </p>
              <p className="mono small selectable">
                {breakLockTarget.old_path}
              </p>
              <p className="muted small">
                {t("repair.breakLockNote")}
              </p>
            </>
          }
          onCancel={() => setBreakLockTarget(null)}
          onConfirm={() => runBreakLock(breakLockTarget)}
        />
      )}
    </Wrapper>
  );
}
