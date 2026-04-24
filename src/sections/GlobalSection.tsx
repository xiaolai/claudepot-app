import { ConfigSection } from "./ConfigSection";

/**
 * Global section — user-wide Claude Code configuration.
 *
 * Wraps `ConfigSection` with `forcedAnchor = { kind: "global" }` so
 * the tree shows only user-level artifacts (User, Global config,
 * Plugins, Memory across projects, Managed policy). No project walk,
 * no anchor picker, no Effective/MCP/Memory panes that depend on a
 * project cwd — the backend's global-only mode rejects those by
 * design.
 *
 * The project-scoped equivalent lives inside the Projects shell's
 * Config tab.
 */
export function GlobalSection({
  subRoute,
  onSubRouteChange,
}: {
  subRoute: string | null;
  onSubRouteChange: (next: string | null) => void;
}) {
  return (
    <ConfigSection
      subRoute={subRoute}
      onSubRouteChange={onSubRouteChange}
      forcedAnchor={{ kind: "global" }}
    />
  );
}
