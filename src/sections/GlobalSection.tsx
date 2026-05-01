import { useState } from "react";
import { ConfigSection } from "./ConfigSection";
import { UpdatesPanel } from "./global/UpdatesPanel";
import { Button } from "../components/primitives/Button";

/**
 * Global section — user-wide Claude Code surfaces.
 *
 * Two tabs:
 *   - **Config** (default) — wraps `ConfigSection` with
 *     `forcedAnchor = { kind: "global" }` so the tree shows only
 *     user-level artifacts (User config, Plugins, Memory across
 *     projects, Managed policy). The project-scoped equivalent
 *     lives inside the Projects shell's Config tab.
 *   - **Updates** — Claude Code CLI + Claude Desktop update manager
 *     (see `src/sections/global/UpdatesPanel.tsx` and
 *     `dev-docs/auto-updates.md`).
 *
 * Tab choice persists in localStorage. We don't reuse `subRoute`
 * for tab selection so ConfigSection's existing sub-routing
 * (effective-settings, effective-mcp, repair, maintenance) stays
 * untouched.
 */

type GlobalTab = "config" | "updates";
const TAB_STORAGE_KEY = "claudepot.global.tab";

function loadTab(): GlobalTab {
  try {
    return localStorage.getItem(TAB_STORAGE_KEY) === "updates"
      ? "updates"
      : "config";
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
        style={{
          display: "flex",
          gap: "var(--sp-4)",
          padding: "var(--sp-8) var(--sp-14) var(--sp-4)",
          borderBottom: "var(--bw-hair) solid var(--line)",
        }}
      >
        <Button
          size="sm"
          variant={tab === "config" ? "subtle" : "ghost"}
          active={tab === "config"}
          onClick={() => switchTab("config")}
        >
          Config
        </Button>
        <Button
          size="sm"
          variant={tab === "updates" ? "subtle" : "ghost"}
          active={tab === "updates"}
          onClick={() => switchTab("updates")}
        >
          Updates
        </Button>
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        {tab === "config" ? (
          <ConfigSection
            subRoute={subRoute}
            onSubRouteChange={onSubRouteChange}
            forcedAnchor={{ kind: "global" }}
          />
        ) : (
          <UpdatesPanel />
        )}
      </div>
    </div>
  );
}
