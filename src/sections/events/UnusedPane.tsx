// Activities → Usage → "Unused" pane.
//
// Split out of UsageView so that file stays inside the loc-guardian
// limit, and because this pane has a genuinely different data source:
// the other three chips share the `artifactUsageTop` rollup, while
// this one needs the installed inventory minus the durable ever-fired
// ledger.
//
// Mounted only while the chip is active — it triggers a filesystem
// config scan, far heavier than the shared rollup.

import { useEffect, useMemo } from "react";
import { UnusedTable } from "./UnusedTable";
import { useUnusedArtifacts } from "./useUnusedArtifacts";

type KindFilter = "all" | "skill" | "hook" | "agent" | "command" | "mcp";

/**
 * The Unused view. Own component so its data fetch is mounted only
 * while the chip is active — it triggers a filesystem config scan,
 * which is far heavier than the shared usage rollup.
 *
 * Wording is deliberate: "no invocation on record", never "never
 * used". The ledger covers what Claudepot observed, so usage predating
 * it — or living only in since-deleted transcripts — is invisible.
 */
export function UnusedPane({
  kind,
  plugin,
  onNavigate,
  onPluginsDiscovered,
}: {
  kind: KindFilter;
  plugin: string;
  /** Shell navigation, used by the per-row "Reveal in Config" action. */
  onNavigate: (id: string, subRoute?: string | null) => void;
  /**
   * Report the plugins owning unused artifacts back to the parent.
   *
   * The shared plugin dropdown is built from the *usage* rollup — i.e.
   * plugins that HAVE fired. Unused rows are the complement, so a
   * plugin whose artifacts have all gone unused would never appear in
   * the dropdown and could not be filtered to: precisely the plugin a
   * user would want to isolate here.
   */
  onPluginsDiscovered: (plugins: string[]) => void;
}) {
  const {
    rows,
    installedCount,
    suppressedRecent,
    suppressedDisabled,
    graceDays,
    loading,
    error,
  } = useUnusedArtifacts(true);

  // Feed the parent's dropdown. Depends on `rows` only, and `rows`
  // changes only when the fetch resolves, so this cannot loop with the
  // parent's state update.
  useEffect(() => {
    const seen = new Set<string>();
    for (const r of rows) if (r.plugin_id) seen.add(r.plugin_id);
    onPluginsDiscovered(Array.from(seen).sort());
    // `onPluginsDiscovered` is wrapped in useCallback by the parent.
  }, [rows, onPluginsDiscovered]);

  const visible = useMemo(() => {
    let out = rows;
    if (kind !== "all") out = out.filter((r) => r.kind === kind);
    if (plugin !== "all") out = out.filter((r) => r.plugin_id === plugin);
    return out;
  }, [rows, kind, plugin]);

  if (loading && rows.length === 0) {
    return <EmptyHint>Scanning installed artifacts…</EmptyHint>;
  }
  if (error) return <EmptyHint danger>Failed to load: {error}</EmptyHint>;
  if (rows.length === 0) {
    return (
      <EmptyHint>
        Every installed artifact has an invocation on record.
      </EmptyHint>
    );
  }
  if (visible.length === 0) {
    // Hooks can never appear here, so "try a different filter" would
    // send the user hunting for rows that cannot exist. State the
    // reason inline instead (rules/design.md).
    if (kind === "hook") {
      return (
        <EmptyHint>
          Hooks aren't listed here. One settings file declares many
          hooks, so there's no per-file identity to match against usage
          records — see the Hot and Noisy views for hooks that did fire.
        </EmptyHint>
      );
    }
    if (kind === "mcp") {
      return (
        <EmptyHint>
          MCP servers aren't listed here. This view compares installed
          artifact <em>files</em> against usage, and an MCP server is a
          configured process rather than a file — see the Hot and Noisy
          views for servers that did fire.
        </EmptyHint>
      );
    }
    return <EmptyHint>No matching artifacts. Try a different filter.</EmptyHint>;
  }

  return (
    <>
      <UnusedTable
        rows={visible}
        onReveal={(r) => onNavigate("global", `node:${r.node_id}`)}
      />
      {/* Inline disclosure, not a tooltip: rules/design.md requires the
          reason next to the thing, and the coverage boundary is a
          correctness caveat rather than a hint. */}
      <p
        style={{
          padding: "var(--sp-8) var(--sp-16)",
          color: "var(--fg-dim)",
          fontSize: "var(--fs-xs)",
          margin: 0,
        }}
      >
        {rows.length} of {installedCount} installed artifacts have no
        invocation on record.
        {suppressedRecent > 0
          ? ` ${suppressedRecent} modified in the last ${graceDays} days ${suppressedRecent === 1 ? "is" : "are"} held back — too new to judge.`
          : ""}
        {suppressedDisabled > 0
          ? ` ${suppressedDisabled} belong${suppressedDisabled === 1 ? "s" : ""} to plugins that aren't enabled and can't fire at all.`
          : ""}{" "}
        Records begin when Claudepot first indexed this machine; earlier
        runs aren't counted. Plugins whose value comes from a bundled MCP
        server won't show fires here — MCP calls aren't tracked as
        artifacts.
      </p>
    </>
  );
}


function EmptyHint({
  children,
  danger = false,
}: {
  children: React.ReactNode;
  danger?: boolean;
}) {
  return (
    <div
      style={{
        padding: "var(--sp-24) var(--sp-16)",
        color: danger ? "var(--danger)" : "var(--fg-dim)",
        fontSize: "var(--fs-xs)",
      }}
    >
      {children}
    </div>
  );
}
