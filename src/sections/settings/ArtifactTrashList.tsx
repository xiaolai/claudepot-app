// Trash list with state-aware actions:
//   Healthy           → Restore (one-click; suffix retry on conflict)
//   MissingManifest   → Recover… (prompt for confirmed target + kind)
//   AbandonedStaging  → Recover…
//   MissingPayload    → Forget only
//   OrphanPayload     → Forget only
// Plus an Empty old button that purges Healthy entries past the
// configured retention.

import { useCallback, useState } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { NF } from "../../icons";
import { formatRelative } from "../../lib/formatRelative";
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
  if (rows === null) {
    return (
      <Section title="Artifact trash">
        <Empty>Loading…</Empty>
      </Section>
    );
  }
  if (rows.length === 0) {
    return (
      <Section title="Artifact trash">
        <Empty>
          Trash is empty. Trashed artifacts are restorable here for
          ~{PURGE_AFTER_DAYS} days.
        </Empty>
      </Section>
    );
  }
  const sorted = [...rows].sort(
    (a, b) => (b.trashed_at_ms ?? 0) - (a.trashed_at_ms ?? 0),
  );
  return (
    <Section
      title={`Artifact trash (${rows.length})`}
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
            <Th>Kind</Th>
            <Th>Name</Th>
            <Th>Trashed</Th>
            <Th>State</Th>
            <Th aria-label="Actions" />
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
  const [busy, setBusy] = useState(false);
  const [confirmForget, setConfirmForget] = useState(false);
  const [confirmSuffix, setConfirmSuffix] = useState(false);
  const [recoverOpen, setRecoverOpen] = useState(false);
  const m = row.manifest;
  const kind = m?.kind ?? "—";
  const name = m?.relative_path ?? `(unrecoverable: ${row.state})`;

  const onRestore = useCallback(async () => {
    setBusy(true);
    try {
      const r = await api.artifactRestoreFromTrash(row.id, "refuse");
      pushToast("info", `Restored to ${r.final_path}`);
      onChanged();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      if (/already exists/i.test(msg)) {
        setConfirmSuffix(true);
      } else {
        pushToast("error", `Restore failed: ${msg}`);
      }
    } finally {
      setBusy(false);
    }
  }, [row, pushToast, onChanged]);

  const restoreWithSuffix = useCallback(async () => {
    setConfirmSuffix(false);
    setBusy(true);
    try {
      const r = await api.artifactRestoreFromTrash(row.id, "suffix");
      pushToast("info", `Restored to ${r.final_path}`);
      onChanged();
    } catch (err) {
      pushToast(
        "error",
        `Restore failed: ${err instanceof Error ? err.message : String(err)}`,
      );
    } finally {
      setBusy(false);
    }
  }, [row, pushToast, onChanged]);

  const doForget = useCallback(async () => {
    setConfirmForget(false);
    setBusy(true);
    try {
      await api.artifactForgetTrash(row.id);
      pushToast("info", "Trash entry forgotten");
      onChanged();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      pushToast("error", `Forget failed: ${msg}`);
    } finally {
      setBusy(false);
    }
  }, [row, pushToast, onChanged]);

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
        pushToast("info", `Recovered to ${r.final_path}`);
        onChanged();
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        pushToast("error", `Recover failed: ${msg}`);
      } finally {
        setBusy(false);
      }
    },
    [row, pushToast, onChanged],
  );

  const trashedAt = row.trashed_at_ms
    ? formatRelative(row.trashed_at_ms, { ago: true })
    : "unknown";

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
              <Button
                variant="ghost"
                glyph={NF.refresh}
                onClick={onRestore}
                disabled={busy}
                size="sm"
              >
                Restore
              </Button>
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
                Recover…
              </Button>
            )}
            <Button
              variant="ghost"
              danger
              glyph={NF.trash}
              onClick={() => setConfirmForget(true)}
              disabled={busy}
              size="sm"
            >
              Forget
            </Button>
          </span>
        </Td>
      </Tr>
      {confirmForget && (
        <ConfirmDialog
          title="Forget this trash entry?"
          body="This entry will be removed from disk. There's no further undo."
          confirmLabel="Forget"
          confirmDanger
          onConfirm={doForget}
          onCancel={() => setConfirmForget(false)}
        />
      )}
      {confirmSuffix && (
        <ConfirmDialog
          title="Original location is occupied"
          body={`Restore "${name}" with a "-N" suffix instead?`}
          confirmLabel="Restore with suffix"
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
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState(false);

  const doPurge = useCallback(async () => {
    setConfirm(false);
    setBusy(true);
    try {
      const n = await api.artifactPurgeTrash(PURGE_AFTER_DAYS);
      pushToast("info", `Purged ${n} entr${n === 1 ? "y" : "ies"}`);
      onChanged();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      pushToast("error", `Purge failed: ${msg}`);
    } finally {
      setBusy(false);
    }
  }, [pushToast, onChanged]);

  return (
    <>
      <Button
        variant="ghost"
        glyph={NF.trash}
        onClick={() => setConfirm(true)}
        disabled={busy}
        size="sm"
        title={`Purge Healthy entries older than ${PURGE_AFTER_DAYS} days`}
      >
        Empty old
      </Button>
      {confirm && (
        <ConfirmDialog
          title={`Purge old trash entries?`}
          body={`Healthy entries older than ${PURGE_AFTER_DAYS} days will be removed. Corrupt entries are kept until manually forgotten.`}
          confirmLabel="Purge"
          confirmDanger
          onConfirm={doPurge}
          onCancel={() => setConfirm(false)}
        />
      )}
    </>
  );
}

function StateBadge({ state }: { state: TrashEntryDto["state"] }) {
  const tone =
    state === "healthy"
      ? "var(--fg-faint)"
      : state === "abandoned_staging"
        ? "var(--warn)"
        : // missing_*, orphan_payload, tampered — all destructive states
          "var(--danger)";
  return (
    <span
      style={{
        fontSize: "var(--fs-2xs)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
        color: tone,
      }}
      title={
        state === "tampered"
          ? "Payload differs from the byte count / sha256 the manifest recorded — restore refused; investigate or forget."
          : undefined
      }
    >
      {state.replace(/_/g, " ")}
    </span>
  );
}
