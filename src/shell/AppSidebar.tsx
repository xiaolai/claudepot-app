import { useState } from "react";
import { Divider } from "../components/primitives/Divider";
import { Glyph } from "../components/primitives/Glyph";
import { SectionLabel } from "../components/primitives/SectionLabel";
import { SidebarItem } from "../components/primitives/SidebarItem";
import { NF } from "../icons";
import type { SectionDef } from "../sections/registry";
import type { AccountSummary } from "../types";
import {
  SidebarTargetSwitcher,
  type SwapTargetId,
} from "./SidebarTargetSwitcher";

interface AppSidebarProps {
  sections: readonly SectionDef[];
  active: string;
  onSelect: (sectionId: string) => void;

  /** Accounts for the two swap-target dropdowns. */
  accounts: AccountSummary[];
  /** Bound UUIDs for each target — null when unbound. */
  binding: { cli: string | null; desktop: string | null };
  onBind: (target: SwapTargetId, uuid: string) => void;

  /** Counts shown on the right side of primary-nav rows. */
  badges?: Partial<Record<string, number>>;

  /** Small text shown below the sync dot. */
  version?: string;
  /** Whether the sync is healthy — drives the dot color. */
  synced?: boolean;
}

const FILESYSTEM_ROWS: {
  id: string;
  glyph: string;
  label: string;
  targetSection?: string;
}[] = [
  { id: "fs-projects", glyph: NF.folder, label: "projects/", targetSection: "projects" },
  { id: "fs-todos", glyph: NF.check, label: "todos/" },
  { id: "fs-shellsnap", glyph: NF.terminal, label: "shell-snapshots/" },
  { id: "fs-commands", glyph: NF.bolt, label: "commands/" },
  { id: "fs-agents", glyph: NF.cpu, label: "agents/" },
  { id: "fs-mcp", glyph: NF.server, label: "mcp.json", targetSection: "settings" },
  { id: "fs-md", glyph: NF.fileMd, label: "CLAUDE.md", targetSection: "settings" },
];

/**
 * Left 240px column: swap-target switchers at top, primary nav in the
 * middle, filesystem tree below, sync strip at the bottom. The
 * "~/.claude" tree entries currently deep-link into existing screens
 * where we have a counterpart; entries without a counterpart do
 * nothing for now (we'll flesh them out once the backend exposes the
 * underlying surfaces).
 */
export function AppSidebar({
  sections,
  active,
  onSelect,
  accounts,
  binding,
  onBind,
  badges,
  version,
  synced = true,
}: AppSidebarProps) {
  const targets = [
    { id: "cli" as const, label: "CLI", glyph: NF.terminal },
    { id: "desktop" as const, label: "Desktop", glyph: NF.users },
  ];

  return (
    <aside
      style={{
        width: "var(--sidebar-width)",
        flexShrink: 0,
        borderRight: "var(--bw-hair) solid var(--line)",
        background: "var(--bg)",
        display: "flex",
        flexDirection: "column",
        overflow: "hidden",
      }}
    >
      {/* two swap targets — CLI + Desktop */}
      <div
        style={{
          padding: "var(--sp-14) var(--sp-10) var(--sp-10)",
        }}
      >
        <SectionLabel
          style={{ padding: "0 var(--sp-4) var(--sp-6)" }}
        >
          Swap targets
        </SectionLabel>
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-4)",
          }}
        >
          {targets.map((t) => (
            <SidebarTargetSwitcher
              key={t.id}
              target={t}
              accounts={accounts}
              boundUuid={binding[t.id]}
              onBind={onBind}
              onManage={() => onSelect("accounts")}
            />
          ))}
        </div>
      </div>

      <Divider style={{ margin: "var(--sp-4) var(--sp-12)" }} />

      {/* primary nav */}
      <div style={{ padding: "var(--sp-4) var(--sp-8)" }}>
        {sections.map((s) => (
          <SidebarItem
            key={s.id}
            glyph={s.glyph}
            label={s.label}
            active={active === s.id}
            badge={badges?.[s.id] ?? undefined}
            onClick={() => onSelect(s.id)}
          />
        ))}
      </div>

      <Divider style={{ margin: "var(--sp-8) var(--sp-12)" }} />

      <SectionLabel>~/.claude</SectionLabel>
      <div style={{ padding: "0 var(--sp-8)" }}>
        {FILESYSTEM_ROWS.map((row) => (
          <FsTreeRow
            key={row.id}
            glyph={row.glyph}
            label={row.label}
            onClick={
              row.targetSection
                ? () => onSelect(row.targetSection!)
                : undefined
            }
            title={
              row.targetSection
                ? undefined
                : "File-browser view not yet implemented"
            }
          />
        ))}
      </div>

      <div style={{ flex: 1 }} />

      {/* sync strip */}
      <div
        style={{
          padding: "var(--sp-10) var(--sp-14)",
          borderTop: "var(--bw-hair) solid var(--line)",
          fontSize: "var(--fs-xs)",
          color: "var(--fg-faint)",
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
        }}
      >
        <Glyph
          g={NF.dot}
          color={synced ? "var(--ok)" : "var(--warn)"}
          style={{ fontSize: "var(--fs-4xs)" }}
        />
        <span>{synced ? "synced" : "syncing…"}</span>
        <span style={{ flex: 1 }} />
        {version && <span>{version}</span>}
      </div>
    </aside>
  );
}

/**
 * Filesystem-tree row under `~/.claude`. Intentionally demoted so it
 * reads as reference, not navigation — smaller font, faint color, no
 * background fill, no left border. Clickable rows get a subtle color
 * bump on hover so the target ones still afford a press; informational
 * rows stay flat.
 */
function FsTreeRow({
  glyph,
  label,
  onClick,
  title,
}: {
  glyph: string;
  label: string;
  onClick?: () => void;
  title?: string;
}) {
  const [hover, setHover] = useState(false);
  const clickable = onClick !== undefined;
  return (
    <button
      type="button"
      title={title}
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      disabled={!clickable}
      style={{
        width: "100%",
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-8)",
        padding: "var(--sp-3) var(--sp-10) var(--sp-3) var(--sp-24)",
        fontSize: "var(--fs-xs)",
        fontWeight: 400,
        color:
          clickable && hover
            ? "var(--fg-muted)"
            : "var(--fg-faint)",
        background: "transparent",
        border: "none",
        textAlign: "left",
        cursor: clickable ? "pointer" : "default",
        transition: "color var(--dur-fast) var(--ease-linear)",
      }}
    >
      <Glyph
        g={glyph}
        color="var(--fg-ghost)"
        style={{ fontSize: "var(--fs-2xs)" }}
      />
      <span
        style={{
          flex: 1,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {label}
      </span>
    </button>
  );
}
