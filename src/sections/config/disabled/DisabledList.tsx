// Left column of the Disabled scope view: groups by kind + scope_root,
// per-row Re-enable and Trash actions. Sharded out of
// DisabledScopeView so each shard stays under the loc-guardian limit.

import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../../api";
import { Button } from "../../../components/primitives/Button";
import { IconButton } from "../../../components/primitives/IconButton";
import { ConfirmDialog } from "../../../components/ConfirmDialog";
import { NF } from "../../../icons";
import { i18n } from "../../../lib/i18n";
import { renderError } from "../../../lib/i18n-error";
import type { DisabledRecordDto, LifecycleKind } from "../../../types";

// Catalog keys, not literals — the group heading is resolved where the
// block renders, so a language switch reaches a list already on screen.
const KINDS = [
  { key: "skill", labelKey: "disabled.kindSkills" },
  { key: "agent", labelKey: "disabled.kindAgents" },
  { key: "command", labelKey: "disabled.kindCommands" },
] as const satisfies readonly { key: LifecycleKind; labelKey: string }[];

interface KindGroup {
  kind: LifecycleKind;
  labelKey: (typeof KINDS)[number]["labelKey"];
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
  const out: KindGroup[] = KINDS.map(({ key, labelKey }) => ({
    kind: key,
    labelKey,
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
  const { t } = useTranslation("config");
  const label = t(group.labelKey);
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
        <span style={{ flex: 1 }}>{label}</span>
        <span>{total}</span>
      </header>
      <ul
        role="listbox"
        aria-label={t("disabled.listAria", { kind: label.toLowerCase() })}
        style={{ listStyle: "none", margin: 0, padding: 0 }}
      >
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
          // Non-interactive subgroup label inside the listbox — keep it
          // out of the option list for assistive tech.
          role="presentation"
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
  const { t } = useTranslation("config");
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
        t("disabled.reenabledToast", {
          kind: record.kind,
          name: record.name,
        }),
      );
      onChanged();
    } catch (err) {
      pushToast("error", renderError(err, t("errors.reenable")));
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
        t("disabled.trashedToast", { kind: record.kind, name: record.name }),
      );
      onChanged();
    } catch (err) {
      pushToast("error", renderError(err, t("errors.trash")));
    } finally {
      setBusy(false);
    }
  }, [record, projectRoot, pushToast, onChanged, t]);

  return (
    <>
      <li
        // Listbox option pattern (design.md a11y floor) — same shape
        // as ProjectsList rows: keyboard-activatable, aria-selected,
        // and a left accent bar so selection isn't conveyed by
        // background color alone.
        role="option"
        aria-selected={selected}
        tabIndex={0}
        className="pm-focus"
        onClick={onSelect}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onSelect();
          }
        }}
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-6)",
          padding: "var(--sp-6) var(--sp-12)",
          background: selected ? "var(--bg-active)" : "transparent",
          color: selected ? "var(--accent-ink)" : "var(--fg)",
          borderLeft: selected
            ? "2px solid var(--accent-border)"
            : "2px solid transparent",
          borderBottom: "var(--bw-hair) solid var(--line)",
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
          {/* Tier 3 (icon-buttons.md): "Disable / Enable" verbs have no
              universal glyph — the label IS the affordance. */}
          <Button variant="ghost" onClick={onEnable} disabled={busy} size="sm">
            {t("disabled.reenable")}
          </Button>
          {/* Tier 1: trash in a dense list row is a universal verb. */}
          <IconButton
            glyph={NF.trash}
            onClick={() => setConfirmTrash(true)}
            disabled={busy}
            size="sm"
            title={t("disabled.moveToTrash")}
            aria-label={t("disabled.moveToTrash")}
          />
        </span>
      </li>
      {confirmTrash && (
        <ConfirmDialog
          title={t("disabled.trashTitle", { kind: record.kind })}
          body={t("disabled.trashBody", { name: record.name })}
          confirmLabel={t("disabled.moveToTrash")}
          confirmDanger
          onConfirm={doTrash}
          onCancel={() => setConfirmTrash(false)}
        />
      )}
    </>
  );
}

function scopeShort(scope: string | undefined, root: string): string {
  if (scope === "user") return i18n.t("disabled.scopeUser", { ns: "config" });
  if (scope === "project") {
    // Strip trailing `/.claude` (Unix) or `\.claude` (Windows) so the
    // row shows the repo path, not the trailing config subdir.
    const trimmed = root.replace(/[/\\]\.claude[/\\]?$/, "");
    return i18n.t("disabled.scopeProject", { ns: "config", path: trimmed });
  }
  return root;
}
