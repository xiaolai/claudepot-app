import { Trans, useTranslation } from "react-i18next";
import { ConfirmDangerousAction } from "../../components/ConfirmDangerousAction";
import type { JournalEntry } from "../../types";

type PendingAction =
  | { kind: "resume"; entry: JournalEntry }
  | { kind: "rollback"; entry: JournalEntry }
  | { kind: "abandon"; entry: JournalEntry };

export type { PendingAction };

export function RepairConfirmDialogs({
  pending,
  onCancel,
  onResume,
  onRollback,
  onAbandon,
}: {
  pending: PendingAction | null;
  onCancel: () => void;
  onResume: (entry: JournalEntry) => void;
  onRollback: (entry: JournalEntry) => void;
  onAbandon: (entry: JournalEntry) => void;
}) {
  const { t } = useTranslation("projects");
  if (!pending) return null;

  if (pending.kind === "resume") {
    return (
      <ConfirmDangerousAction
        title={t("repair.resumeConfirmTitle")}
        confirmLabel={t("repair.resume")}
        danger={false}
        consequences={
          <>
            <p>{t("repair.resumeBody")}</p>
            <p className="mono small muted">
              {pending.entry.old_path} → {pending.entry.new_path}
            </p>
          </>
        }
        onCancel={onCancel}
        onConfirm={() => onResume(pending.entry)}
      />
    );
  }

  if (pending.kind === "rollback") {
    return (
      <ConfirmDangerousAction
        title={t("repair.rollbackConfirmTitle")}
        confirmLabel={t("repair.rollback")}
        consequences={
          <>
            <p>{t("repair.rollbackBody")}</p>
            {pending.entry.snapshot_paths.length > 0 && (
              <div className="muted small">
                <strong>{t("repair.snapshotsLabel")}</strong>
                <ul>
                  {pending.entry.snapshot_paths.map((s) => (
                    <li key={s} className="mono">{s}</li>
                  ))}
                </ul>
                {t("repair.snapshotsNote")}
              </div>
            )}
          </>
        }
        onCancel={onCancel}
        onConfirm={() => onRollback(pending.entry)}
      />
    );
  }

  return (
    <ConfirmDangerousAction
      title={t("repair.abandonConfirmTitle")}
      confirmLabel={t("repair.abandon")}
      // Localized, like the retention gate's phrase. The token is the
      // one thing in this dialog the user must reproduce, so leaving it
      // as the English literal asked a reader of a fully-Chinese dialog
      // to type a word it never showed them. AGENTS.md's "don't
      // localize what the user types" rule exists to keep paths and
      // commands typeable — a confirmation token is the documented
      // exception, because the friction only works if the word is read.
      typeToConfirm={t("repair.abandonPhrase")}
      consequences={
        <>
          <p>
            <Trans
              ns="projects"
              i18nKey="repair.abandonBody"
              components={{ f: <code className="mono">.abandoned.json</code> }}
            />
          </p>
          <p className="muted small">
            {t("repair.abandonNote")}
          </p>
        </>
      }
      onCancel={onCancel}
      onConfirm={() => onAbandon(pending.entry)}
    />
  );
}
