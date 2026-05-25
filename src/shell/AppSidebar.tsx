import type React from "react";
import { Divider } from "../components/primitives/Divider";
import { Glyph } from "../components/primitives/Glyph";
import { SectionLabel } from "../components/primitives/SectionLabel";
import { SidebarItem } from "../components/primitives/SidebarItem";
import { NF } from "../icons";
import type { SectionDef } from "../sections/registry";
import type { AccountSummary, LiveSessionSummary } from "../types";
import { SidebarBgBadge } from "./SidebarBgBadge";
import { SidebarLiveStrip } from "./SidebarLiveStrip";
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

  /** Trailing content for primary-nav rows. Numbers render as plain
   *  counts; any other ReactNode (e.g. "Off" chip) renders verbatim. */
  badges?: Partial<Record<string, React.ReactNode>>;

  /** Small text shown below the sync dot. */
  version?: string;
  /** Whether the sync is healthy — drives the dot color. */
  synced?: boolean;

  /** Invoked when a user activates a row in the live Activity strip.
   * The parent routes to the Sessions deep-link (M1) or the live
   * pane (M2+). Optional so existing callers needn't change. */
  onOpenLiveSession?: (session: LiveSessionSummary) => void;

  /**
   * Rail-width state. When true the sidebar shrinks to icon-only
   * (~52 px). Swap-targets, activity, and sync strips hide; nav rows
   * become glyph-only. Toggle is the chevron at the very bottom.
   */
  collapsed?: boolean;
  /** Called when the user clicks the collapse/expand chevron. */
  onToggleCollapsed?: () => void;
}

/**
 * Left 240px column: swap-target switchers at top, primary nav in the
 * middle, live Activity strip (render-if-nonzero), sync strip at the
 * bottom.
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
  onOpenLiveSession,
  collapsed = false,
  onToggleCollapsed,
}: AppSidebarProps) {
  const targets = [
    { id: "cli" as const, label: "CLI", glyph: NF.terminal },
    { id: "desktop" as const, label: "Desktop", glyph: NF.desktop },
  ];

  return (
    <aside
      data-collapsed={collapsed || undefined}
      style={{
        width: collapsed ? "var(--rail-max-width)" : "var(--sidebar-width)",
        flexShrink: 0,
        borderRight: "var(--bw-hair) solid var(--line)",
        background: "var(--bg)",
        display: "flex",
        flexDirection: "column",
        overflow: "hidden",
        transition: "width var(--dur-fast) var(--ease-out)",
      }}
    >
      {/* Swap targets — full picker when expanded, hidden when
          collapsed. The dropdowns need their email labels to be
          useful at all; reducing them to two unlabelled glyphs would
          hide which account is currently bound, which is the whole
          point of the panel. */}
      {!collapsed && (
        <>
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
        </>
      )}

      {/* primary nav */}
      <div
        style={{
          padding: collapsed
            ? "var(--sp-14) var(--sp-4) var(--sp-4)"
            : "var(--sp-4) var(--sp-8)",
        }}
      >
        {sections.map((s) => (
          <SidebarItem
            key={s.id}
            glyph={s.glyph}
            label={s.label}
            active={active === s.id}
            badge={badges?.[s.id] ?? undefined}
            onClick={() => onSelect(s.id)}
            collapsed={collapsed}
            title={collapsed ? s.label : undefined}
          />
        ))}
      </div>

      {!collapsed && (
        <Divider style={{ margin: "var(--sp-8) var(--sp-12)" }} />
      )}

      {/* Live Activity strip — render-if-nonzero, so the divider
          and label disappear together when no sessions are active.
          Hidden in collapsed mode since session previews need width. */}
      {!collapsed && onOpenLiveSession && (
        <SidebarLiveStrip onOpenSession={onOpenLiveSession} />
      )}

      {/* Background-worker chip — render-if-nonzero. CC's per-user
          supervisor holds detached `/bg` sessions; this surfaces the
          count so utilization burn isn't invisible.
          See dev-docs/cc-daemon-research.md. */}
      <SidebarBgBadge collapsed={collapsed} />

      <div style={{ flex: 1 }} />

      {/* Sync + collapse strip. Expanded: dot · "synced" · version ·
          chevron. Collapsed: dot + chevron stacked, no text. */}
      <div
        style={{
          padding: collapsed
            ? "var(--sp-8) var(--sp-4)"
            : "var(--sp-10) var(--sp-14)",
          borderTop: "var(--bw-hair) solid var(--line)",
          fontSize: "var(--fs-xs)",
          color: "var(--fg-faint)",
          display: "flex",
          flexDirection: collapsed ? "column" : "row",
          alignItems: "center",
          gap: collapsed ? "var(--sp-6)" : "var(--sp-8)",
        }}
      >
        <Glyph
          g={NF.dot}
          color={synced ? "var(--ok)" : "var(--warn)"}
          style={{ fontSize: "var(--fs-4xs)" }}
        />
        {!collapsed && (
          <>
            <span>{synced ? "synced" : "syncing…"}</span>
            <span style={{ flex: 1 }} />
            {version && <span>{version}</span>}
          </>
        )}
        {onToggleCollapsed && (
          <button
            type="button"
            onClick={onToggleCollapsed}
            title={
              collapsed
                ? "Expand sidebar (⌘\\)"
                : "Collapse sidebar (⌘\\)"
            }
            aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
            aria-expanded={!collapsed}
            className="pm-focus"
            style={{
              display: "inline-flex",
              alignItems: "center",
              justifyContent: "center",
              width: "var(--sp-22)",
              height: "var(--sp-22)",
              padding: 0,
              marginLeft: collapsed ? 0 : "var(--sp-4)",
              background: "transparent",
              border: "var(--bw-hair) solid transparent",
              borderRadius: "var(--r-1)",
              color: "var(--fg-muted)",
              cursor: "pointer",
            }}
          >
            <Glyph
              g={collapsed ? NF.chevronR : NF.chevronL}
              style={{ fontSize: "var(--fs-xs)" }}
            />
          </button>
        )}
      </div>
    </aside>
  );
}
