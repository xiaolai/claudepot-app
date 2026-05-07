// Search input + filter chips + the rendered list of TipRows.
//
// Filters are single-select. Counts come from the backend.

import { useMemo, useState } from "react";
import { Button } from "../../../components/primitives/Button";
import type { RenderedTip, TipsCounts } from "../../../types/cc-tips";
import { TipRow } from "./TipRow";

type Filter = "all" | "seen" | "never-seen" | "active-experiments";

export function TipsList({
  tips,
  counts,
}: {
  tips: RenderedTip[];
  counts: TipsCounts;
}) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return tips.filter((t) => {
      if (filter === "seen" && t.seen_status !== "seen") return false;
      if (filter === "never-seen" && t.seen_status !== "never-seen")
        return false;
      if (filter === "active-experiments" && !t.experiment_flag) return false;
      if (q.length === 0) return true;
      return (
        t.id.toLowerCase().includes(q) ||
        t.category_label.toLowerCase().includes(q) ||
        t.trigger_summary.toLowerCase().includes(q) ||
        t.prose.toLowerCase().includes(q) ||
        (t.prose_b ?? "").toLowerCase().includes(q)
      );
    });
  }, [tips, query, filter]);

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0 }}>
      <div
        style={{
          display: "flex",
          gap: "var(--sp-8)",
          padding: "var(--sp-10) var(--sp-14)",
          alignItems: "center",
          flexWrap: "wrap",
          borderBottom: "var(--bw-hair) solid var(--line)",
        }}
      >
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Filter tips by prose, id, or trigger…"
          style={{
            flex: "1 1 tokens.config.cmd.col.max",
            minWidth: 240,
            padding: "var(--sp-4) var(--sp-8)",
            fontFamily: "var(--font-mono)",
            fontSize: "var(--fs-xs)",
            background: "var(--bg-raised)",
            border: "var(--bw-hair) solid var(--line-strong)",
            borderRadius: "var(--rad-2)",
            color: "var(--fg)",
          }}
        />
        <Chip
          active={filter === "all"}
          label={`All ${counts.all}`}
          onClick={() => setFilter("all")}
        />
        <Chip
          active={filter === "seen"}
          label={`Seen ${counts.seen}`}
          onClick={() => setFilter("seen")}
        />
        <Chip
          active={filter === "never-seen"}
          label={`Never seen ${counts.never_seen}`}
          onClick={() => setFilter("never-seen")}
        />
        <Chip
          active={filter === "active-experiments"}
          label={`Experiments ${counts.active_experiments}`}
          onClick={() => setFilter("active-experiments")}
        />
      </div>
      <div style={{ flex: 1, overflow: "auto", minHeight: 0 }}>
        {filtered.length === 0 ? (
          <div
            style={{
              padding: "var(--sp-16)",
              fontSize: "var(--fs-xs)",
              color: "var(--fg-faint)",
              textAlign: "center",
            }}
          >
            No tips match the current filter.
          </div>
        ) : (
          filtered.map((t) => <TipRow key={t.id} tip={t} />)
        )}
      </div>
    </div>
  );
}

function Chip({
  active,
  label,
  onClick,
}: {
  active: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <Button
      size="sm"
      variant={active ? "subtle" : "ghost"}
      active={active}
      onClick={onClick}
    >
      {label}
    </Button>
  );
}
