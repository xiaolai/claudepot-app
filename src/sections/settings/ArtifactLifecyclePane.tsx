// Settings → Cleanup → Artifacts.
//
// Two stacked sections in a single pane:
//   1. Disabled artifacts — with Re-enable / Trash per row
//   2. Trash — with Restore (Healthy) / Recover (corrupt) /
//      Forget (any) / Empty old (purge older than 30d)
//
// Renders nothing when both lists are empty (the user has no
// artifact lifecycle activity yet); the parent decides how to label
// "no entries" cases.

import { useCallback, useEffect, useState } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import { formatRelative } from "../../lib/formatRelative";
import type {
  DisabledRecordDto,
  LifecycleKind,
  TrashEntryDto,
} from "../../types";

const PURGE_AFTER_DAYS = 30;

export function ArtifactLifecyclePane({
  pushToast,
  projectRoot,
}: {
  pushToast: (kind: "info" | "error", text: string) => void;
  /** Active project anchor, when one is set. Forwarded to
   * `artifact_list_disabled` so project-scoped disabled artifacts
   * surface alongside user-scoped ones. */
  projectRoot: string | null;
}) {
  const [disabled, setDisabled] = useState<DisabledRecordDto[] | null>(null);
  const [trash, setTrash] = useState<TrashEntryDto[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [refreshTick, setRefreshTick] = useState(0);

  const refresh = useCallback(() => setRefreshTick((n) => n + 1), []);

  useEffect(() => {
    let cancelled = false;
    setLoadError(null);
    Promise.all([
      api.artifactListDisabled(projectRoot),
      api.artifactListTrash(),
    ])
      .then(([d, t]) => {
        if (cancelled) return;
        setDisabled(d);
        setTrash(t);
      })
      .catch((err) => {
        if (cancelled) return;
        setLoadError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [projectRoot, refreshTick]);

  // One-time auto-purge of old Healthy entries on mount. Quiet —
  // toast only when something is actually purged.
  useEffect(() => {
    let cancelled = false;
    api
      .artifactPurgeTrash(PURGE_AFTER_DAYS)
      .then((n) => {
        if (cancelled || n === 0) return;
        pushToast("info", `Auto-purged ${n} trash entr${n === 1 ? "y" : "ies"} older than ${PURGE_AFTER_DAYS} days`);
        refresh();
      })
      .catch(() => {
        // Silent: cleanup is best-effort.
      });
    return () => {
      cancelled = true;
    };
    // refresh and pushToast are stable from the parent.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (loadError) {
    return (
      <Section title="Artifacts">
        <Empty danger>Couldn't load artifact lifecycle: {loadError}</Empty>
      </Section>
    );
  }

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-24)",
      }}
    >
      <DisabledList
        rows={disabled}
        projectRoot={projectRoot}
        pushToast={pushToast}
        onChanged={refresh}
      />
      <TrashList
        rows={trash}
        pushToast={pushToast}
        onChanged={refresh}
      />
    </div>
  );
}

function DisabledList({
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

  const onTrash = useCallback(async () => {
    if (
      !window.confirm(
        `Move ${record.kind} "${record.name}" to trash? You can restore it within ~30 days.`,
      )
    ) {
      return;
    }
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
            onClick={onTrash}
            disabled={busy}
            size="sm"
          >
            Trash
          </Button>
        </span>
      </Td>
    </tr>
  );
}

function TrashList({
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
      // Conflict → ask to suffix on retry.
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
    // Inline prompt for the confirmed target. Real implementation
    // would use a Modal — keeping this slice tight, the prompt is
    // good enough for the rare-corruption flow.
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

// --- presentational primitives -----------------------------------

function Section({
  title,
  action,
  children,
}: {
  title: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section>
      <header
        style={{
          display: "flex",
          alignItems: "baseline",
          justifyContent: "space-between",
          gap: "var(--sp-12)",
          marginBottom: "var(--sp-8)",
        }}
      >
        <h3
          style={{
            margin: 0,
            fontSize: "var(--fs-sm)",
            fontWeight: 600,
            color: "var(--fg)",
          }}
        >
          {title}
        </h3>
        {action}
      </header>
      {children}
    </section>
  );
}

function Table({ children }: { children: React.ReactNode }) {
  return (
    <table
      style={{
        width: "100%",
        borderCollapse: "collapse",
        fontSize: "var(--fs-sm)",
        fontVariantNumeric: "tabular-nums",
      }}
    >
      {children}
    </table>
  );
}

function Th({ children, ...rest }: React.ThHTMLAttributes<HTMLTableCellElement>) {
  return (
    <th
      {...rest}
      style={{
        textAlign: "left",
        padding: "var(--sp-6) var(--sp-12)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        fontWeight: 500,
        color: "var(--fg-muted)",
        fontSize: "var(--fs-2xs)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
      }}
    >
      {children}
    </th>
  );
}

function Td({
  children,
  muted,
  align,
}: {
  children: React.ReactNode;
  muted?: boolean;
  align?: "left" | "right";
}) {
  return (
    <td
      style={{
        padding: "var(--sp-6) var(--sp-12)",
        textAlign: align ?? "left",
        color: muted ? "var(--fg-muted)" : "var(--fg)",
      }}
    >
      {children}
    </td>
  );
}

function rowStyle(): React.CSSProperties {
  return { borderBottom: "var(--bw-hair) solid var(--line)" };
}

function Empty({
  children,
  danger,
}: {
  children: React.ReactNode;
  danger?: boolean;
}) {
  return (
    <div
      style={{
        padding: "var(--sp-16) var(--sp-20)",
        background: "var(--bg-sunken)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        fontSize: "var(--fs-sm)",
        color: danger ? "var(--danger)" : "var(--fg-muted)",
      }}
    >
      <Glyph g={NF.info} style={{ marginRight: "var(--sp-6)" }} />
      {children}
    </div>
  );
}
