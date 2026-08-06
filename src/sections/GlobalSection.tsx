import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ConfigSection } from "./ConfigSection";
import { UpdatesPanel } from "./global/UpdatesPanel";
import { MemoryHealthPanel } from "./global/MemoryHealthPanel";
import { TipsPanel } from "./global/TipsPanel";
import { Button } from "../components/primitives/Button";

/**
 * Global section — user-wide Claude Code surfaces.
 *
 * Tabs (in tab-bar order):
 *   - **Config** (default) — wraps `ConfigSection` with
 *     `forcedAnchor = { kind: "global" }` so the tree shows only
 *     user-level artifacts (User config, Plugins, Memory across
 *     projects, Managed policy). The project-scoped equivalent
 *     lives inside the Projects shell's Config tab.
 *   - **Memory** — `~/.claude/CLAUDE.md` health and across-projects
 *     memory dashboard.
 *   - **Updates** — Claude Code CLI + Claude Desktop update manager
 *     (see `src/sections/global/UpdatesPanel.tsx` and
 *     `dev-docs/auto-updates.md`).
 *
 * Tab choice persists in localStorage. We don't reuse `subRoute`
 * for tab selection so ConfigSection's existing sub-routing
 * (effective-settings, effective-mcp, repair, maintenance) stays
 * untouched.
 */

import { GLOBAL_TAB_KEY as TAB_STORAGE_KEY } from "../lib/storageKeys";
import {
  EVENT_GLOBAL_TAB,
  consumeGlobalTabHint,
  type GlobalTabEventDetail,
} from "../lib/networkPanelDeepLink";
import { GLOBAL_TABS, isGlobalTabId, type GlobalTabId } from "./global/tabs";

type GlobalTab = GlobalTabId;

function loadTab(): GlobalTab {
  // A ⌘K deep link wins over the saved tab on a cold mount; it is a
  // one-shot hint, so reading it also clears it.
  const hint = consumeGlobalTabHint();
  if (hint && isGlobalTabId(hint)) return hint;
  try {
    const raw = localStorage.getItem(TAB_STORAGE_KEY);
    if (raw && isGlobalTabId(raw)) return raw;
    return "config";
  } catch {
    return "config";
  }
}

function saveTab(t: GlobalTab) {
  try {
    localStorage.setItem(TAB_STORAGE_KEY, t);
  } catch {
    /* ignore quota / private mode */
  }
}

export function GlobalSection({
  subRoute,
  onSubRouteChange,
}: {
  subRoute: string | null;
  onSubRouteChange: (next: string | null) => void;
}) {
  const { t } = useTranslation("global");
  const [tab, setTab] = useState<GlobalTab>(loadTab);

  // A `node:<id>` subRoute is a deep-link into the Config tree (e.g.
  // "Reveal in Config" from Activities -> Usage -> Unused). The tab
  // otherwise restores from localStorage, so without this the link
  // would land on whichever tab was last open and silently do nothing.
  //
  // It fires only when the route ACTUALLY CHANGES, not whenever a
  // `node:` value is present. `node:<id>` doubles as the Config tree's
  // persisted selection (ConfigSection writes it on every node click),
  // so `setSection("global")` restores one from localStorage on any
  // later visit. Forcing Config on that restore made the ⌘K "Open
  // Global → Updates" link land on Config for anyone who had ever
  // selected a Config node.
  const prevSubRoute = useRef<string | null | undefined>(undefined);
  useEffect(() => {
    const changed =
      prevSubRoute.current !== undefined && prevSubRoute.current !== subRoute;
    prevSubRoute.current = subRoute;
    if (changed && subRoute?.startsWith("node:") && tab !== "config") {
      setTab("config");
      saveTab("config");
    }
  }, [subRoute, tab]);

  const switchTab = (next: GlobalTab) => {
    setTab(next);
    saveTab(next);
  };

  // Hot-mount half of the ⌘K tab deep link ("Open Global → Updates").
  // `setSection` is a no-op when Global is already the active section,
  // so the cold-mount hint in `loadTab` never re-fires; the event
  // reaches us either way. Mirrors SettingsSection's pane deep link.
  useEffect(() => {
    const handler = (e: Event) => {
      const next = (e as CustomEvent<GlobalTabEventDetail>).detail?.tab;
      if (next && isGlobalTabId(next)) {
        setTab(next);
        saveTab(next);
      }
    };
    window.addEventListener(EVENT_GLOBAL_TAB, handler);
    return () => window.removeEventListener(EVENT_GLOBAL_TAB, handler);
  }, []);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div
        role="tablist"
        aria-label={t("tabs.ariaLabel")}
        style={{
          display: "flex",
          gap: "var(--sp-4)",
          padding: "var(--sp-8) var(--sp-14) var(--sp-4)",
          borderBottom: "var(--bw-hair) solid var(--line)",
        }}
      >
        {/* Rendered from GLOBAL_TABS so the tab bar and the ⌘K deep
            links can't disagree about which tabs exist or their
            order — they already had, with Tips and Updates swapped. */}
        {/* Named `def`, not `t` — `t` is the translator in this scope. */}
        {GLOBAL_TABS.map((def) => (
          <Button
            key={def.id}
            id={`global-tab-${def.id}`}
            role="tab"
            aria-selected={tab === def.id}
            aria-controls={`global-panel-${def.id}`}
            size="sm"
            variant={tab === def.id ? "subtle" : "ghost"}
            active={tab === def.id}
            onClick={() => switchTab(def.id)}
          >
            {def.label}
          </Button>
        ))}
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        {tab === "config" && (
          <div
            role="tabpanel"
            id="global-panel-config"
            aria-labelledby="global-tab-config"
            style={{ height: "100%" }}
          >
            <ConfigSection
              subRoute={subRoute}
              onSubRouteChange={onSubRouteChange}
              forcedAnchor={{ kind: "global" }}
            />
          </div>
        )}
        {tab === "updates" && (
          <div
            role="tabpanel"
            id="global-panel-updates"
            aria-labelledby="global-tab-updates"
          >
            <UpdatesPanel
              // Setting the sub-route is enough to land on the pane:
              // the effect above already forces the Config tab for any
              // `node:*` route, so this needs no tab plumbing of its own.
              onEditEnvVars={() => onSubRouteChange("node:virtual:env-vars")}
            />
          </div>
        )}
        {tab === "memory" && (
          <div
            role="tabpanel"
            id="global-panel-memory"
            aria-labelledby="global-tab-memory"
          >
            <MemoryHealthPanel />
          </div>
        )}
        {tab === "tips" && (
          <div
            role="tabpanel"
            id="global-panel-tips"
            aria-labelledby="global-tab-tips"
            style={{ height: "100%" }}
          >
            <TipsPanel />
          </div>
        )}
      </div>
    </div>
  );
}
