// Trash list with state-aware actions:
//   Healthy           → Restore (one-click; suffix retry on conflict)
//   MissingManifest   → Recover… (prompt for confirmed target + kind)
//   AbandonedStaging  → Recover…
//   MissingPayload    → Forget only
//   OrphanPayload     → Forget only
// Plus an Empty old button that purges Healthy entries past the
// configured retention.

import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { IconButton } from "../../components/primitives/IconButton";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { NF } from "../../icons";
import { formatRelative } from "../../lib/formatRelative";
import { extractMessage, renderError } from "../../lib/i18n-error";
import type { LifecycleKind, TrashEntryDto } from "../../types";
import { Table, Th, Td, Tr } from "../../components/primitives";
import { Section, Empty } from "./LifecyclePresentational";
import { RecoverDialog } from "./RecoverDialog";

export const PURGE_AFTER_DAYS = 30;

export function ArtifactTrashList({
  rows,
  pushToast,
  onChanged,
}: {
  rows: TrashEntryDto[] | null;
  pushToast: (kind: "info" | "error", text: string) => void;
  onChanged: () => void;
}) {
  const { t } = useTranslation("settings");
  if (rows === null) {
    return (
      <Section title={t("trash.title")}>
        <Empty>{t("shared.loading")}</Empty>
      </Section>
    );
  }
  if (rows.length === 0) {
    return (
      <Section title={t("trash.title")}>
        <Empty>{t("trash.empty", { days: PURGE_AFTER_DAYS })}</Empty>
      </Section>
    );
  }
  const sorted = [...rows].sort(
    (a, b) => (b.trashed_at_ms ?? 0) - (a.trashed_at_ms ?? 0),
  );
  return (
    <Section
      title={t("trash.titleCount", { count: rows.length })}
      action={
        <PurgeButton
          pushToast={pushToast}
          onChanged={onChanged}
          rowCount={rows.length}
        />
      }
    >
      <Table>
        <thead>
          <tr>
            <Th>{t("artifacts.thKind")}</Th>
            <Th>{t("artifacts.thName")}</Th>
            <Th>{t("artifacts.thTrashed")}</Th>
            <Th>{t("artifacts.thState")}</Th>
            <Th aria-label={t("artifacts.thActions")} />
          </tr>
        </thead>
        <tbody>
          {sorted.map((row) => (
            <TrashRow
              key={row.id}
              row={row}
              pushToast={pushToast}
              onChanged={onChanged}
            />
          ))}
        </tbody>
      </Table>
    </Section>
  );
}

function TrashRow({
  row,
  pushToast,
  onChanged,
}: {
  row: TrashEntryDto;
  pushToast: (kind: "info" | "error", text: string) => void;
  onChanged: () => void;
}) {
  const { t } = useTranslation("settings");
  const [busy, setBusy] = useState(false);
  const [confirmForget, setConfirmForget] = useState(false);
  const [confirmSuffix, setConfirmSuffix] = useState(false);
  const [recoverOpen, setRecoverOpen] = useState(false);
  const m = row.manifest;
  const kind = m?.kind ?? "—";
  const name =
    m?.relative_path ?? t("trash.unrecoverable", { state: row.state });

  const onRestore = useCallback(async () => {
    setBusy(true);
    try {
      const r = await api.artifactRestoreFromTrash(row.id, "refuse");
      pushToast("info", t("trash.restoredTo", { path: r.final_path }));
      onChanged();
    } catch (err) {
      // Raw (untruncated, unredacted) text — this is a control-flow
      // test, not a display string. `renderError` would cap it at 240
      // chars and could push the marker out of range.
      const msg = extractMessage(err);
      if (/already exists/i.test(msg)) {
        setConfirmSuffix(true);
      } else {
        pushToast("error", renderError(err, t("trash.restoreFailed")));
      }
    } finally {
      setBusy(false);
    }
  }, [row, pushToast, onChanged, t]);

  const restoreWithSuffix = useCallback(async () => {
    setConfirmSuffix(false);
    setBusy(true);
    try {
      const r = await api.artifactRestoreFromTrash(row.id, "suffix");
      pushToast("info", t("trash.restoredTo", { path: r.final_path }));
      onChanged();
    } catch (err) {
      pushToast("error", renderError(err, t("trash.restoreFailed")));
    } finally {
      setBusy(false);
    }
  }, [row, pushToast, onChanged, t]);

  const doForget = useCallback(async () => {
    setConfirmForget(false);
    setBusy(true);
    try {
      await api.artifactForgetTrash(row.id);
      pushToast("info", t("trash.forgotten"));
      onChanged();
    } catch (err) {
      pushToast("error", renderError(err, t("trash.forgetFailed")));
    } finally {
      setBusy(false);
    }
  }, [row, pushToast, onChanged, t]);

  const doRecover = useCallback(
    async (target: string, recoveryKind: LifecycleKind) => {
      setRecoverOpen(false);
      setBusy(true);
      try {
        const r = await api.artifactRecoverTrash(
          row.id,
          target,
          recoveryKind,
          "refuse",
        );
        pushToast("info", t("trash.recoveredTo", { path: r.final_path }));
        onChanged();
      } catch (err) {
        pushToast("error", renderError(err, t("trash.recoverFailed")));
      } finally {
        setBusy(false);
      }
    },
    [row, pushToast, onChanged, t],
  );

  const trashedAt = row.trashed_at_ms
    ? formatRelative(row.trashed_at_ms, { ago: true })
    : t("trash.unknown");

  return (
    <>
      <Tr>
        <Td muted>{kind}</Td>
        <Td>
          <span style={{ fontWeight: 500 }} title={m?.original_path ?? row.entry_dir}>
            {name}
          </span>
        </Td>
        <Td muted>{trashedAt}</Td>
        <Td muted>
          <StateBadge state={row.state} />
        </Td>
        <Td align="right">
          <span style={{ display: "inline-flex", gap: "var(--sp-6)" }}>
            {row.state === "healthy" && (
              <IconButton
                glyph={NF.refresh}
                onClick={onRestore}
                disabled={busy}
                size="sm"
                title={t("trash.restore")}
                aria-label={t("trash.restore")}
              />
            )}
            {(row.state === "missing_manifest" ||
              row.state === "abandoned_staging") && (
              <Button
                variant="ghost"
                glyph={NF.refresh}
                onClick={() => setRecoverOpen(true)}
                disabled={busy}
                size="sm"
              >
                {t("trash.recover")}
              </Button>
            )}
            <IconButton
              glyph={NF.trash}
              onClick={() => setConfirmForget(true)}
              disabled={busy}
              size="sm"
              title={t("trash.forgetTitle")}
              aria-label={t("trash.forgetTitle")}
            />
          </span>
        </Td>
      </Tr>
      {confirmForget && (
        <ConfirmDialog
          title={t("trash.confirmForget.title")}
          body={t("trash.confirmForget.body")}
          confirmLabel={t("trash.confirmForget.confirm")}
          confirmDanger
          onConfirm={doForget}
          onCancel={() => setConfirmForget(false)}
        />
      )}
      {confirmSuffix && (
        <ConfirmDialog
          title={t("trash.confirmSuffix.title")}
          body={t("trash.confirmSuffix.body", { name })}
          confirmLabel={t("trash.confirmSuffix.confirm")}
          onConfirm={restoreWithSuffix}
          onCancel={() => setConfirmSuffix(false)}
        />
      )}
      {recoverOpen && (
        <RecoverDialog
          entry={row}
          onCancel={() => setRecoverOpen(false)}
          onSubmit={doRecover}
        />
      )}
    </>
  );
}

function PurgeButton({
  pushToast,
  onChanged,
  rowCount: _rowCount,
}: {
  pushToast: (kind: "info" | "error", text: string) => void;
  onChanged: () => void;
  rowCount: number;
}) {
  const { t } = useTranslation("settings");
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState(false);

  const doPurge = useCallback(async () => {
    setConfirm(false);
    setBusy(true);
    try {
      const n = await api.artifactPurgeTrash(PURGE_AFTER_DAYS);
      pushToast("info", t("trash.purged", { count: n }));
      onChanged();
    } catch (err) {
      pushToast("error", renderError(err, t("trash.purgeFailed")));
    } finally {
      setBusy(false);
    }
  }, [pushToast, onChanged, t]);

  return (
    <>
      <Button
        variant="ghost"
        glyph={NF.trash}
        onClick={() => setConfirm(true)}
        disabled={busy}
        size="sm"
        title={t("trash.purgeTitle", { days: PURGE_AFTER_DAYS })}
      >
        {t("trash.emptyOld")}
      </Button>
      {confirm && (
        <ConfirmDialog
          title={t("trash.confirmPurge.title")}
          body={t("trash.confirmPurge.body", { days: PURGE_AFTER_DAYS })}
          confirmLabel={t("trash.confirmPurge.confirm")}
          confirmDanger
          onConfirm={doPurge}
          onCancel={() => setConfirm(false)}
        />
      )}
    </>
  );
}

function StateBadge({ state }: { state: TrashEntryDto["state"] }) {
  const { t } = useTranslation("settings");
  const tone =
    state === "healthy"
      ? "var(--fg-faint)"
      : state === "abandoned_staging"
        ? "var(--warn)"
        : // missing_*, orphan_payload, tampered — all destructive states
          "var(--danger)";
  const stateLabels: Record<TrashEntryDto["state"], string> = {
    healthy: t("trash.state.healthy"),
    missing_manifest: t("trash.state.missing_manifest"),
    missing_payload: t("trash.state.missing_payload"),
    orphan_payload: t("trash.state.orphan_payload"),
    abandoned_staging: t("trash.state.abandoned_staging"),
    tampered: t("trash.state.tampered"),
  };
  return (
    <span
      style={{
        fontSize: "var(--fs-2xs)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
        color: tone,
      }}
      title={state === "tampered" ? t("trash.tamperedTitle") : undefined}
    >
      {stateLabels[state] ?? state.replace(/_/g, " ")}
    </span>
  );
}
