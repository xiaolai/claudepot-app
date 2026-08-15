// Activity → Usage subview.
//
// Sortable per-artifact rollup answering "what's earning its keep?"
// — the inverse of the Cards stream which answers "what broke?"
//
// Data source: `api.artifactUsageTop(null, 500)` on mount and on
// `live-all` events (coalesced behind a trailing 2 s debounce —
// the backend only publishes on real state changes, but a busy
// session can still emit several per second), plus a 5s polling
// fallback so newly-fired events surface without a manual refresh.
//
// Filters:
//   Kind chips      — All / Skill / Hook / Agent / Command / MCP
//   Plugin dropdown — populated from the data
//   View chips      — Hot (top 20 by 7d) / Noisy (≥10% errors)
//                     / Unused (installed, no invocation on record) / All
//
// The "Unused" view is the follow-up this file's header previously
// deferred. It does NOT share the `artifactUsageTop` rollup: it needs
// the installed inventory (Config tree) minus the durable ever-fired
// ledger, which is a different query pair. That lives in
// `useUnusedArtifacts`, gated on the chip being active so the
// filesystem scan isn't paid for by the other three views.
//
// Hooks are absent from Unused by construction — `artifactKeyForFile`
// documents that hooks are 1:N with files and have no per-file key, so
// they can't be missed against the ledger.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";
import type { ArtifactUsageRowDto } from "../../types";
import { UsageTable, type SortKey } from "./UsageTable";
import { UnusedPane } from "./UnusedPane";

type KindFilter = "all" | "skill" | "hook" | "agent" | "command" | "mcp";
type ViewMode = "hot" | "noisy" | "unused" | "all";

const POLL_MS = 5000;
// Min interval between live-driven refetches — coalesces a burst of
// `live-all` events into one 500-row fetch. Matches the debounce in
// EventsSection so the two surfaces share one freshness contract.
const LIVE_REFRESH_DEBOUNCE_MS = 2000;

interface UsageViewProps {
  /** Shell navigation — forwarded to the Unused view's Reveal action. */
  onNavigate?: (id: string, subRoute?: string | null) => void;
  /**
   * Optional callback the parent invokes to register UsageView's
   * refresh function. Lets the EventsSection header refresh button
   * trigger an immediate fetch when the user is on the Usage tab —
   * otherwise it would only refresh the (hidden) card stream.
   * Pass `null` on unmount to clear the registration.
   */
  registerRefresh?: (refresh: (() => void) | null) => void;
}

export function UsageView({ registerRefresh, onNavigate }: UsageViewProps = {}) {
  const { t } = useTranslation("activities");
  const [rows, setRows] = useState<ArtifactUsageRowDto[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [kind, setKind] = useState<KindFilter>("all");
  const [plugin, setPlugin] = useState<string | "all">("all");
  const [view, setView] = useState<ViewMode>("hot");
  const [sortKey, setSortKey] = useState<SortKey>("count_30d");
  const seqRef = useRef(0);

  const refresh = useCallback(async () => {
    const seq = ++seqRef.current;
    try {
      const next = await api.artifactUsageTop(null, 500);
      if (seq !== seqRef.current) return;
      setRows(next);
      setError(null);
    } catch (e) {
      if (seq !== seqRef.current) return;
      setError(renderError(e));
    } finally {
      if (seq === seqRef.current) setLoading(false);
    }
  }, []);

  // Trailing debounce: the first event of a burst schedules one
  // refetch; events inside the window are absorbed. The handle lives
  // in a ref shared with the poll effect's cleanup below, which
  // clears any pending debounce on unmount or refresh-identity
  // rotation (exactly what the old single-effect teardown did).
  const liveDebounceRef = useRef<number | null>(null);
  useTauriEvent("live-all", () => {
    if (liveDebounceRef.current !== null) return;
    liveDebounceRef.current = window.setTimeout(() => {
      liveDebounceRef.current = null;
      void refresh();
    }, LIVE_REFRESH_DEBOUNCE_MS);
  });
  useEffect(() => {
    void refresh();
    const interval = window.setInterval(() => void refresh(), POLL_MS);
    return () => {
      window.clearInterval(interval);
      if (liveDebounceRef.current !== null) {
        window.clearTimeout(liveDebounceRef.current);
        liveDebounceRef.current = null;
      }
    };
  }, [refresh]);

  useEffect(() => {
    if (!registerRefresh) return;
    registerRefresh(() => void refresh());
    return () => registerRefresh(null);
  }, [registerRefresh, refresh]);

  // Plugins owning UNUSED artifacts, reported up by UnusedPane. Kept
  // separate from the usage rollup because the two sets are
  // complements: an artifact is in one or the other, never both. The
  // dropdown unions them so a plugin whose artifacts have all gone
  // unused is still selectable.
  const [unusedPlugins, setUnusedPlugins] = useState<string[]>([]);
  const handleUnusedPlugins = useCallback((next: string[]) => {
    // Replace only on a real change — UnusedPane reports on every
    // fetch, and an unconditional setState would re-render the parent
    // (and thus remount nothing, but churn) each poll.
    setUnusedPlugins((prev) =>
      prev.length === next.length && prev.every((p, i) => p === next[i])
        ? prev
        : next,
    );
  }, []);

  const plugins = useMemo(() => {
    const seen = new Set<string>();
    for (const r of rows) if (r.plugin_id) seen.add(r.plugin_id);
    for (const p of unusedPlugins) seen.add(p);
    return Array.from(seen).sort();
  }, [rows, unusedPlugins]);

  const visible = useMemo(() => {
    let out = rows;
    if (kind !== "all") out = out.filter((r) => r.kind === kind);
    if (plugin !== "all") out = out.filter((r) => r.plugin_id === plugin);
    if (view === "noisy") {
      out = out.filter(
        (r) =>
          r.stats.error_count_30d > 0 &&
          r.stats.error_count_30d / Math.max(r.stats.count_30d, 1) >= 0.1,
      );
    }
    out = [...out].sort((a, b) => {
      const av = sortValue(a, sortKey);
      const bv = sortValue(b, sortKey);
      if (av === bv) return a.artifact_key.localeCompare(b.artifact_key);
      return bv - av;
    });
    if (view === "hot") {
      // Top 20 by 7d count after filtering — the sort above already
      // ordered by `sortKey`; for Hot we override to an explicit 7d
      // re-sort then truncate so the chip's promise (recent winners)
      // holds even when the user changed sortKey.
      out = [...out]
        .sort((a, b) => b.stats.count_7d - a.stats.count_7d)
        .filter((r) => r.stats.count_7d > 0)
        .slice(0, 20);
    }
    return out;
  }, [rows, kind, plugin, view, sortKey]);

  return (
    <div
      style={{
        flex: 1,
        display: "flex",
        flexDirection: "column",
        minHeight: 0,
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
          padding: "var(--sp-8) var(--sp-16)",
          borderBottom: "var(--bw-hair) solid var(--line)",
          flexWrap: "wrap",
        }}
      >
        <KindChip current={kind} onPick={setKind} value="all" label={t("usage.kindAll")} />
        <KindChip current={kind} onPick={setKind} value="skill" label={t("usage.kindSkills")} />
        <KindChip current={kind} onPick={setKind} value="hook" label={t("usage.kindHooks")} />
        <KindChip current={kind} onPick={setKind} value="agent" label={t("usage.kindAgents")} />
        <KindChip current={kind} onPick={setKind} value="command" label={t("usage.kindCommands")} />
        <KindChip current={kind} onPick={setKind} value="mcp" label={t("usage.kindMcp")} />
        <span style={{ flex: 1 }} />
        <ViewChip current={view} onPick={setView} value="hot" label={t("usage.viewHot")} />
        <ViewChip current={view} onPick={setView} value="noisy" label={t("usage.viewNoisy")} />
        <ViewChip current={view} onPick={setView} value="unused" label={t("usage.viewUnused")} />
        <ViewChip current={view} onPick={setView} value="all" label={t("usage.viewAll")} />
        <select
          aria-label={t("usage.filterByPlugin")}
          value={plugin}
          onChange={(e) => setPlugin(e.target.value as string)}
          style={{
            padding: "var(--sp-4) var(--sp-8)",
            background: "var(--bg)",
            color: "var(--fg)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-1)",
            fontSize: "var(--fs-xs)",
          }}
        >
          <option value="all">{t("usage.allPlugins")}</option>
          {plugins.map((p) => (
            <option key={p} value={p}>
              {p}
            </option>
          ))}
        </select>
      </div>

      <div style={{ flex: 1, overflow: "auto", minHeight: 0 }}>
        {view === "unused" ? (
          <UnusedPane
            kind={kind}
            plugin={plugin}
            onNavigate={onNavigate ?? (() => {})}
            onPluginsDiscovered={handleUnusedPlugins}
          />
        ) : loading && rows.length === 0 ? (
          <EmptyHint>{t("usage.loading")}</EmptyHint>
        ) : error ? (
          <EmptyHint danger>{t("usage.loadFailed", { error })}</EmptyHint>
        ) : visible.length === 0 ? (
          <EmptyHint>
            {t("usage.noMatch")}{" "}
            {rows.length === 0
              ? t("usage.runSession")
              : t("usage.tryDifferentFilter")}
          </EmptyHint>
        ) : (
          <UsageTable
            rows={visible}
            sortKey={sortKey}
            onSort={setSortKey}
          />
        )}
      </div>
    </div>
  );
}

function sortValue(row: ArtifactUsageRowDto, key: SortKey): number {
  const s = row.stats;
  switch (key) {
    case "count_30d":
      return s.count_30d;
    case "count_7d":
      return s.count_7d;
    case "count_24h":
      return s.count_24h;
    case "last_seen":
      return s.last_seen_ms ?? 0;
    case "errors":
      return s.error_count_30d;
    case "p50":
      return s.p50_ms_24h ?? s.avg_ms_30d ?? 0;
  }
}

function KindChip({
  value,
  label,
  current,
  onPick,
}: {
  value: KindFilter;
  label: string;
  current: KindFilter;
  onPick: (v: KindFilter) => void;
}) {
  const active = current === value;
  return (
    <button
      type="button"
      onClick={() => onPick(value)}
      className="pm-focus"
      style={chipStyle(active)}
    >
      {label}
    </button>
  );
}

function ViewChip({
  value,
  label,
  current,
  onPick,
}: {
  value: ViewMode;
  label: string;
  current: ViewMode;
  onPick: (v: ViewMode) => void;
}) {
  const active = current === value;
  return (
    <button
      type="button"
      onClick={() => onPick(value)}
      className="pm-focus"
      style={chipStyle(active)}
    >
      {label}
    </button>
  );
}

function chipStyle(active: boolean) {
  return {
    padding: "var(--sp-4) var(--sp-10)",
    background: active ? "var(--accent-soft)" : "transparent",
    color: active ? "var(--accent-ink)" : "var(--fg-muted)",
    border: `var(--bw-hair) solid ${active ? "var(--accent-border)" : "var(--line)"}`,
    borderRadius: "var(--r-1)",
    fontSize: "var(--fs-xs)",
  } as const;
}

function EmptyHint({
  children,
  danger,
}: {
  children: React.ReactNode;
  danger?: boolean;
}) {
  return (
    <div
      style={{
        padding: "var(--sp-24)",
        textAlign: "center",
        color: danger ? "var(--danger)" : "var(--fg-faint)",
        fontSize: "var(--fs-sm)",
      }}
    >
      {children}
    </div>
  );
}
