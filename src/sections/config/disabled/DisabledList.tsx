// Left column of the Disabled scope view: groups by kind + scope_root,
// per-row Re-enable and Trash actions. Sharded out of
// DisabledScopeView so each shard stays under the loc-guardian limit.

import { useCallback, useState } from "react";
import { api } from "../../../api";
import { Button } from "../../../components/primitives/Button";
import { ConfirmDialog } from "../../../components/ConfirmDialog";
import { NF } from "../../../icons";
import type { DisabledRecordDto, LifecycleKind } from "../../../types";

const KINDS: ReadonlyArray<{ key: LifecycleKind; label: string }> = [
  { key: "skill", label: "Skills" },
  { key: "agent", label: "Agents" },
  { key: "command", label: "Commands" },
];

interface KindGroup {
  kind: LifecycleKind;
  label: string;
  byRoot: Map<string, DisabledRecordDto[]>;
}

export function rowKey(r: DisabledRecordDto): string {
  return `${r.scope_root}|${r.kind}|${r.name}`;
}

export function matches(target: DisabledRecordDto): (r: DisabledRecordDto) => boolean {
  return (r) =>
    r.scope_root === target.scope_root &&
    r.kind === target.kind &&
    r.name === target.name;
}

function groupByKindAndRoot(rows: DisabledRecordDto[]): KindGroup[] {
  const out: KindGroup[] = KINDS.map(({ key, label }) => ({
    kind: key,
    label,
    byRoot: new Map(),
  }));
  for (const r of rows) {
    const group = out.find((g) => g.kind === (r.kind as LifecycleKind));
    if (!group) continue;
    const list = group.byRoot.get(r.scope_root) ?? [];
    list.push(r);
    group.byRoot.set(r.scope_root, list);
  }
  for (const g of out) {
    for (const list of g.byRoot.values()) {
      list.sort((a, b) => a.name.localeCompare(b.name));
    }
  }
  return out.filter((g) => g.byRoot.size > 0);
}

export function DisabledList({
  rows,
  selected,
  onSelect,
  projectRoot,
  pushToast,
  onChanged,
}: {
  rows: DisabledRecordDto[];
  selected: DisabledRecordDto | null;
  onSelect: (r: DisabledRecordDto | null) => void;
  projectRoot: string | null;
  pushToast: (kind: "info" | "error", text: string) => void;
  onChanged: () => void;
}) {
  const grouped = groupByKindAndRoot(rows);
  return (
    <div
      style={{
        overflow: "auto",
        minHeight: 0,
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        background: "var(--bg)",
      }}
    >
      {grouped.map((g) => (
        <KindBlock
          key={g.kind}
          group={g}
          selected={selected}
          onSelect={onSelect}
          projectRoot={projectRoot}
          pushToast={pushToast}
          onChanged={onChanged}
        />
      ))}
    </div>
  );
}

function KindBlock({
  group,
  selected,
  onSelect,
  projectRoot,
  pushToast,
  onChanged,
}: {
  group: KindGroup;
  selected: DisabledRecordDto | null;
  onSelect: (r: DisabledRecordDto | null) => void;
  projectRoot: string | null;
  pushToast: (kind: "info" | "error", text: string) => void;
  onChanged: () => void;
}) {
  const total = Array.from(group.byRoot.values()).reduce(
    (n, list) => n + list.length,
    0,
  );
  const showRootSubgroups = group.byRoot.size > 1;
  return (
    <section>
      <header
        style={{
          display: "flex",
          alignItems: "baseline",
          gap: "var(--sp-8)",
          padding: "var(--sp-8) var(--sp-12)",
          background: "var(--bg-sunken)",
          borderBottom: "var(--bw-hair) solid var(--line)",
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-muted)",
          letterSpacing: "var(--ls-wide)",
          textTransform: "uppercase",
          fontWeight: 600,
        }}
      >
        <span style={{ flex: 1 }}>{group.label}</span>
        <span>{total}</span>
      </header>
      <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
        {Array.from(group.byRoot.entries()).map(([root, list]) => (
          <RootSubgroup
            key={root}
            root={root}
            list={list}
            selected={selected}
            onSelect={onSelect}
            projectRoot={projectRoot}
            pushToast={pushToast}
            onChanged={onChanged}
            showRootLabel={showRootSubgroups}
          />
        ))}
      </ul>
    </section>
  );
}

function RootSubgroup({
  root,
  list,
  selected,
  onSelect,
  projectRoot,
  pushToast,
  onChanged,
  showRootLabel,
}: {
  root: string;
  list: DisabledRecordDto[];
  selected: DisabledRecordDto | null;
  onSelect: (r: DisabledRecordDto | null) => void;
  projectRoot: string | null;
  pushToast: (kind: "info" | "error", text: string) => void;
  onChanged: () => void;
  showRootLabel: boolean;
}) {
  return (
    <>
      {showRootLabel && (
        <li
          style={{
            padding: "var(--sp-6) var(--sp-12)",
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            background: "var(--bg)",
            borderBottom: "var(--bw-hair) dashed var(--line)",
          }}
          title={root}
        >
          {scopeShort(list[0]?.scope, root)}
        </li>
      )}
      {list.map((r) => (
        <Row
          key={rowKey(r)}
          record={r}
          selected={selected != null && matches(selected)(r)}
          onSelect={() => onSelect(r)}
          projectRoot={projectRoot}
          pushToast={pushToast}
          onChanged={onChanged}
        />
      ))}
    </>
  );
}

function Row({
  record,
  selected,
  onSelect,
  projectRoot,
  pushToast,
  onChanged,
}: {
  record: DisabledRecordDto;
  selected: boolean;
  onSelect: () => void;
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
      pushToast(
        "error",
        `Re-enable failed: ${err instanceof Error ? err.message : String(err)}`,
      );
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
      pushToast(
        "error",
        `Trash failed: ${err instanceof Error ? err.message : String(err)}`,
      );
    } finally {
      setBusy(false);
    }
  }, [record, projectRoot, pushToast, onChanged]);

  return (
    <>
      <li
        onClick={onSelect}
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-6)",
          padding: "var(--sp-6) var(--sp-12)",
          background: selected ? "var(--bg-active)" : "transparent",
          color: selected ? "var(--accent-ink)" : "var(--fg)",
          borderBottom: "var(--bw-hair) solid var(--line)",
          cursor: "pointer",
          fontSize: "var(--fs-sm)",
        }}
      >
        <span
          style={{
            flex: 1,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
          title={record.current_path}
        >
          {record.name}
        </span>
        <span
          // Wrap so the row click doesn't toggle selection when the
          // user clicks one of the per-row action buttons.
          onClick={(e) => e.stopPropagation()}
          style={{ display: "inline-flex", gap: "var(--sp-4)" }}
        >
          <Button
            variant="ghost"
            glyph={NF.refresh}
            onClick={onEnable}
            disabled={busy}
            size="sm"
            title="Re-enable"
          />
          <Button
            variant="ghost"
            danger
            glyph={NF.trash}
            onClick={() => setConfirmTrash(true)}
            disabled={busy}
            size="sm"
            title="Move to trash"
          />
        </span>
      </li>
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

function scopeShort(scope: string | undefined, root: string): string {
  if (scope === "user") return "User";
  if (scope === "project") {
    const trimmed = root.replace(/\/\.claude\/?$/, "");
    return `Project: ${trimmed}`;
  }
  return root;
}
