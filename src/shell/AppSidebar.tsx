import { Divider } from "../components/primitives/Divider";
import { Glyph } from "../components/primitives/Glyph";
import { SectionLabel } from "../components/primitives/SectionLabel";
import { SidebarItem } from "../components/primitives/SidebarItem";
import { NF } from "../icons";
import type { SectionDef } from "../sections/registry";
import type { AccountSummary, LiveSessionSummary } from "../types";
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

  /** Counts shown on the right side of primary-nav rows. */
  badges?: Partial<Record<string, number>>;

  /** Small text shown below the sync dot. */
  version?: string;
  /** Whether the sync is healthy — drives the dot color. */
  synced?: boolean;

  /** Invoked when a user activates a row in the live Activity strip.
   * The parent routes to the Sessions deep-link (M1) or the live
   * pane (M2+). Optional so existing callers needn't change. */
  onOpenLiveSession?: (session: LiveSessionSummary) => void;
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
}: AppSidebarProps) {
  const targets = [
    { id: "cli" as const, label: "CLI", glyph: NF.terminal },
    { id: "desktop" as const, label: "Desktop", glyph: NF.desktop },
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

      {/* Live Activity strip — render-if-nonzero, so the divider
          and label disappear together when no sessions are active. */}
      {onOpenLiveSession && (
        <SidebarLiveStrip onOpenSession={onOpenLiveSession} />
      )}

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
