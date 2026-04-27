// Per-row Re-enable / Trash actions for the Disabled list inside
// the Settings → Cleanup → Artifacts pane. Sharded out of
// ArtifactLifecyclePane so each surface stays under the loc-guardian
// limit and the table renderer is independently testable.

import { useCallback, useState } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { NF } from "../../icons";
import type { DisabledRecordDto, LifecycleKind } from "../../types";
import { Section, Empty, Table, Th, Td, rowStyle } from "./LifecyclePresentational";

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
  if (rows === null) {
    return (
      <Section title="Disabled artifacts">
        <Empty>Loading…</Empty>
      </Section>
    );
  }
  if (rows.length === 0) {
    return (
      <Section title="Disabled artifacts">
        <Empty>
          Nothing disabled. Use the <em>Disable</em> action in the Config
          tree to hide an artifact from Claude Code without deleting it.
        </Empty>
      </Section>
    );
  }
  return (
    <Section title={`Disabled artifacts (${rows.length})`}>
      <Table>
        <thead>
          <tr>
            <Th>Kind</Th>
            <Th>Name</Th>
            <Th>Scope</Th>
            <Th aria-label="Actions" />
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
      pushToast("info", `Re-enabled ${record.kind} "${record.name}"`);
      onChanged();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      pushToast("error", `Re-enable failed: ${msg}`);
    } finally {
      setBusy(false);
    }
  }, [record, projectRoot, pushToast, onChanged]);

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
      pushToast("info", `Moved ${record.kind} "${record.name}" to trash`);
      onChanged();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      pushToast("error", `Trash failed: ${msg}`);
    } finally {
      setBusy(false);
    }
  }, [record, projectRoot, pushToast, onChanged]);

  return (
    <>
      <tr style={rowStyle()}>
        <Td muted>{record.kind}</Td>
        <Td>
          <span style={{ fontWeight: 500 }} title={record.current_path}>
            {record.name}
          </span>
        </Td>
        <Td muted>
          <span title={record.scope_root}>
            {record.scope === "user" ? "User" : "Project"}
          </span>
        </Td>
        <Td align="right">
          <span style={{ display: "inline-flex", gap: "var(--sp-6)" }}>
            <Button
              variant="ghost"
              glyph={NF.refresh}
              onClick={onEnable}
              disabled={busy}
              size="sm"
            >
              Re-enable
            </Button>
            <Button
              variant="ghost"
              danger
              glyph={NF.trash}
              onClick={() => setConfirmTrash(true)}
              disabled={busy}
              size="sm"
            >
              Trash
            </Button>
          </span>
        </Td>
      </tr>
      {confirmTrash && (
        <ConfirmDialog
          title={`Move ${record.kind} to trash?`}
          body={`"${record.name}" will move to trash. You can restore it within ~30 days.`}
          confirmLabel="Move to trash"
          confirmDanger
          onConfirm={doTrash}
          onCancel={() => setConfirmTrash(false)}
        />
      )}
    </>
  );
}
