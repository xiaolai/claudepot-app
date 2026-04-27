// Activity → Usage subview.
//
// Sortable per-artifact rollup answering "what's earning its keep?"
// — the inverse of the Cards stream which answers "what broke?"
//
// Data source: `api.artifactUsageTop(null, 500)` on mount and on
// every `live-all` tick, plus a 5s polling fallback so newly-fired
// events surface without a manual refresh.
//
// Filters:
//   Kind chips      — All / Skill / Hook / Agent / Command
//   Plugin dropdown — populated from the data
//   View chips      — Hot (top 20 by 7d) / Noisy (≥10% errors) / All
//
// An "Unused" view (joining installed artifacts from `config_view`
// against recorded keys) is a future follow-up; the corresponding
// data API has been removed for now to keep the IPC surface honest
// about what the UI actually consumes.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../../api";
import type { ArtifactUsageRowDto } from "../../types";
import { UsageTable, type SortKey } from "./UsageTable";

type KindFilter = "all" | "skill" | "hook" | "agent" | "command";
type ViewMode = "hot" | "noisy" | "all";

const POLL_MS = 5000;

interface UsageViewProps {
  /**
   * Optional callback the parent invokes to register UsageView's
   * refresh function. Lets the EventsSection header refresh button
   * trigger an immediate fetch when the user is on the Usage tab —
   * otherwise it would only refresh the (hidden) card stream.
   * Pass `null` on unmount to clear the registration.
   */
  registerRefresh?: (refresh: (() => void) | null) => void;
}

export function UsageView({ registerRefresh }: UsageViewProps = {}) {
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
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (seq === seqRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const interval = window.setInterval(() => void refresh(), POLL_MS);
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen("live-all", () => void refresh()).then((u) => {
      if (cancelled) {
        u();
      } else {
        unlisten = u;
      }
    });
    return () => {
      cancelled = true;
      window.clearInterval(interval);
      unlisten?.();
    };
  }, [refresh]);

  useEffect(() => {
    if (!registerRefresh) return;
    registerRefresh(() => void refresh());
    return () => registerRefresh(null);
  }, [registerRefresh, refresh]);

  const plugins = useMemo(() => {
    const seen = new Set<string>();
    for (const r of rows) if (r.plugin_id) seen.add(r.plugin_id);
    return Array.from(seen).sort();
  }, [rows]);

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
        <KindChip current={kind} onPick={setKind} value="all" label="All" />
        <KindChip current={kind} onPick={setKind} value="skill" label="Skills" />
        <KindChip current={kind} onPick={setKind} value="hook" label="Hooks" />
        <KindChip current={kind} onPick={setKind} value="agent" label="Agents" />
        <KindChip current={kind} onPick={setKind} value="command" label="Commands" />
        <span style={{ flex: 1 }} />
        <ViewChip current={view} onPick={setView} value="hot" label="Hot" />
        <ViewChip current={view} onPick={setView} value="noisy" label="Noisy" />
        <ViewChip current={view} onPick={setView} value="all" label="All" />
        <select
          aria-label="Filter by plugin"
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
          <option value="all">All plugins</option>
          {plugins.map((p) => (
            <option key={p} value={p}>
              {p}
            </option>
          ))}
        </select>
      </div>

      <div style={{ flex: 1, overflow: "auto", minHeight: 0 }}>
        {loading && rows.length === 0 ? (
          <EmptyHint>Loading usage data…</EmptyHint>
        ) : error ? (
          <EmptyHint danger>Failed to load: {error}</EmptyHint>
        ) : visible.length === 0 ? (
          <EmptyHint>
            No matching artifacts.{" "}
            {rows.length === 0
              ? "Run a session to populate usage data."
              : "Try a different filter."}
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
    cursor: "pointer",
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
