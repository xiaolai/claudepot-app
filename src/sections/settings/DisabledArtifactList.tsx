// Per-row Re-enable / Trash actions for the Disabled list inside
// the Settings → Cleanup → Artifacts pane. Sharded out of
// ArtifactLifecyclePane so each surface stays under the loc-guardian
// limit and the table renderer is independently testable.

import { useCallback, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { api } from "../../api";
import { IconButton } from "../../components/primitives/IconButton";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { NF } from "../../icons";
import { renderError } from "../../lib/i18n-error";
import type { DisabledRecordDto, LifecycleKind } from "../../types";
import { Table, Th, Td, Tr } from "../../components/primitives";
import { Section, Empty } from "./LifecyclePresentational";

export function DisabledArtifactList({
  rows,
  projectRoot,
  pushToast,
  onChanged,
}: {
  rows: DisabledRecordDto[] | null;
  projectRoot: string | null;
  pushToast: (kind: "info" | "error", text: string) => void;
  onChanged: () => void;
}) {
  const { t } = useTranslation("settings");
  if (rows === null) {
    return (
      <Section title={t("artifacts.disabledTitle")}>
        <Empty>{t("shared.loading")}</Empty>
      </Section>
    );
  }
  if (rows.length === 0) {
    return (
      <Section title={t("artifacts.disabledTitle")}>
        <Empty>
          <Trans
            ns="settings"
            i18nKey="artifacts.noneDisabled"
            components={{ em: <em /> }}
          />
        </Empty>
      </Section>
    );
  }
  return (
    <Section title={t("artifacts.disabledTitleCount", { count: rows.length })}>
      <Table>
        <thead>
          <tr>
            <Th>{t("artifacts.thKind")}</Th>
            <Th>{t("artifacts.thName")}</Th>
            <Th>{t("artifacts.thScope")}</Th>
            <Th aria-label={t("artifacts.thActions")} />
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <DisabledRow
              key={`${r.scope_root}|${r.kind}|${r.name}`}
              record={r}
              projectRoot={projectRoot}
              pushToast={pushToast}
              onChanged={onChanged}
            />
          ))}
        </tbody>
      </Table>
    </Section>
  );
}

function DisabledRow({
  record,
  projectRoot,
  pushToast,
  onChanged,
}: {
  record: DisabledRecordDto;
  projectRoot: string | null;
  pushToast: (kind: "info" | "error", text: string) => void;
  onChanged: () => void;
}) {
  const { t } = useTranslation("settings");
  const [busy, setBusy] = useState(false);
  const [confirmTrash, setConfirmTrash] = useState(false);

  const onEnable = useCallback(async () => {
    setBusy(true);
    try {
      await api.artifactEnable(
        record.scope_root,
        record.kind as LifecycleKind,
        record.name,
        "refuse",
        projectRoot,
      );
      pushToast(
        "info",
        t("artifacts.reenabled", { kind: record.kind, name: record.name }),
      );
      onChanged();
    } catch (err) {
      pushToast("error", renderError(err, t("artifacts.reenableFailed")));
    } finally {
      setBusy(false);
    }
  }, [record, projectRoot, pushToast, onChanged, t]);

  const doTrash = useCallback(async () => {
    setConfirmTrash(false);
    setBusy(true);
    try {
      await api.artifactTrash(
        record.scope_root,
        record.kind as LifecycleKind,
        record.name,
        projectRoot,
      );
      pushToast(
        "info",
        t("artifacts.movedToTrash", { kind: record.kind, name: record.name }),
      );
      onChanged();
    } catch (err) {
      pushToast("error", renderError(err, t("artifacts.trashFailed")));
    } finally {
      setBusy(false);
    }
  }, [record, projectRoot, pushToast, onChanged, t]);

  return (
    <>
      <Tr>
        <Td muted>{record.kind}</Td>
        <Td>
          <span style={{ fontWeight: 500 }} title={record.current_path}>
            {record.name}
          </span>
        </Td>
        <Td muted>
          <span title={record.scope_root}>
            {record.scope === "user"
              ? t("artifacts.scopeUser")
              : t("artifacts.scopeProject")}
          </span>
        </Td>
        <Td align="right">
          <span style={{ display: "inline-flex", gap: "var(--sp-6)" }}>
            <IconButton
              glyph={NF.restore}
              onClick={onEnable}
              disabled={busy}
              size="sm"
              title={t("artifacts.reenable")}
              aria-label={t("artifacts.reenable")}
            />
            <IconButton
              glyph={NF.trash}
              onClick={() => setConfirmTrash(true)}
              disabled={busy}
              size="sm"
              title={t("artifacts.moveToTrash")}
              aria-label={t("artifacts.moveToTrash")}
            />
          </span>
        </Td>
      </Tr>
      {confirmTrash && (
        <ConfirmDialog
          title={t("artifacts.confirmTrash.title", { kind: record.kind })}
          body={t("artifacts.confirmTrash.body", { name: record.name })}
          confirmLabel={t("artifacts.confirmTrash.confirm")}
          confirmDanger
          onConfirm={doTrash}
          onCancel={() => setConfirmTrash(false)}
        />
      )}
    </>
  );
}
