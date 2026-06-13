import { useState } from "react";
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

type GlobalTab = "config" | "updates" | "memory" | "tips";
import { GLOBAL_TAB_KEY as TAB_STORAGE_KEY } from "../lib/storageKeys";

function loadTab(): GlobalTab {
  try {
    const raw = localStorage.getItem(TAB_STORAGE_KEY);
    if (raw === "updates") return "updates";
    if (raw === "memory") return "memory";
    if (raw === "tips") return "tips";
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
  const [tab, setTab] = useState<GlobalTab>(loadTab);

  const switchTab = (next: GlobalTab) => {
    setTab(next);
    saveTab(next);
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div
        role="tablist"
        aria-label="Global views"
        style={{
          display: "flex",
          gap: "var(--sp-4)",
          padding: "var(--sp-8) var(--sp-14) var(--sp-4)",
          borderBottom: "var(--bw-hair) solid var(--line)",
        }}
      >
        <Button
          id="global-tab-config"
          role="tab"
          aria-selected={tab === "config"}
          aria-controls="global-panel-config"
          size="sm"
          variant={tab === "config" ? "subtle" : "ghost"}
          active={tab === "config"}
          onClick={() => switchTab("config")}
        >
          Config
        </Button>
        <Button
          id="global-tab-memory"
          role="tab"
          aria-selected={tab === "memory"}
          aria-controls="global-panel-memory"
          size="sm"
          variant={tab === "memory" ? "subtle" : "ghost"}
          active={tab === "memory"}
          onClick={() => switchTab("memory")}
        >
          Memory
        </Button>
        <Button
          id="global-tab-tips"
          role="tab"
          aria-selected={tab === "tips"}
          aria-controls="global-panel-tips"
          size="sm"
          variant={tab === "tips" ? "subtle" : "ghost"}
          active={tab === "tips"}
          onClick={() => switchTab("tips")}
        >
          Tips
        </Button>
        <Button
          id="global-tab-updates"
          role="tab"
          aria-selected={tab === "updates"}
          aria-controls="global-panel-updates"
          size="sm"
          variant={tab === "updates" ? "subtle" : "ghost"}
          active={tab === "updates"}
          onClick={() => switchTab("updates")}
        >
          Updates
        </Button>
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
            <UpdatesPanel />
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
