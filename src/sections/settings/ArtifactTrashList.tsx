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
import { NF } from "../../icons";
import { formatRelative } from "../../lib/formatRelative";
import type { LifecycleKind, TrashEntryDto } from "../../types";
import { Section, Empty, Table, Th, Td, rowStyle } from "./LifecyclePresentational";

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
        if (
          window.confirm(
            `Original location is occupied. Restore with "-N" suffix?`,
          )
        ) {
          try {
            const r = await api.artifactRestoreFromTrash(row.id, "suffix");
            pushToast("info", `Restored to ${r.final_path}`);
            onChanged();
          } catch (err2) {
            pushToast(
              "error",
              `Restore failed: ${err2 instanceof Error ? err2.message : String(err2)}`,
            );
          }
        }
      } else {
        pushToast("error", `Restore failed: ${msg}`);
      }
    } finally {
      setBusy(false);
    }
  }, [row, pushToast, onChanged]);

  const onForget = useCallback(async () => {
    if (!window.confirm(`Forget this trash entry? It will be gone for good.`)) {
      return;
    }
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

  const onRecover = useCallback(async () => {
    // Prompts are good enough for the rare-corruption flow.
    const guessedKind = m?.kind ?? "agent";
    const guessedTarget = m?.original_path ?? "";
    const target = window.prompt(
      `Recover this entry. Confirm the absolute target path:`,
      guessedTarget,
    );
    if (!target) return;
    const inputKind = window.prompt(
      `Confirm artifact kind (skill / agent / command):`,
      guessedKind,
    );
    if (!inputKind) return;
    if (!["skill", "agent", "command"].includes(inputKind)) {
      pushToast("error", `Unknown kind: ${inputKind}`);
      return;
    }
    setBusy(true);
    try {
      const r = await api.artifactRecoverTrash(
        row.id,
        target,
        inputKind as LifecycleKind,
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
  }, [row, m, pushToast, onChanged]);

  const trashedAt = row.trashed_at_ms
    ? formatRelative(row.trashed_at_ms, { ago: true })
    : "unknown";

  return (
    <tr style={rowStyle()}>
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
              onClick={onRecover}
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
            onClick={onForget}
            disabled={busy}
            size="sm"
          >
            Forget
          </Button>
        </span>
      </Td>
    </tr>
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
  const onClick = useCallback(async () => {
    if (
      !window.confirm(
        `Purge Healthy trash entries older than ${PURGE_AFTER_DAYS} days? Corrupt entries are kept until manually forgotten.`,
      )
    ) {
      return;
    }
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
    <Button
      variant="ghost"
      glyph={NF.trash}
      onClick={onClick}
      disabled={busy}
      size="sm"
      title={`Purge Healthy entries older than ${PURGE_AFTER_DAYS} days`}
    >
      Empty old
    </Button>
  );
}

function StateBadge({ state }: { state: TrashEntryDto["state"] }) {
  const tone =
    state === "healthy"
      ? "var(--fg-faint)"
      : state === "abandoned_staging"
        ? "var(--warn)"
        : "var(--danger)";
  return (
    <span
      style={{
        fontSize: "var(--fs-2xs)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
        color: tone,
      }}
    >
      {state.replace(/_/g, " ")}
    </span>
  );
}
