import { useState } from "react";
import { Button } from "../../components/primitives/Button";
import { Tag } from "../../components/primitives/Tag";
import type { AgentSummaryDto } from "../../types";
import { RunHistoryPanel } from "./RunHistoryPanel";

interface Props {
  agent: AgentSummaryDto;
  busy: boolean;
  /** Increments when a run completes — RunHistoryPanel re-fetches. */
  runsRefreshKey: number;
  onRun: (id: string) => void;
  onEdit: (a: AgentSummaryDto) => void;
  onToggle: (id: string, enabled: boolean) => void;
  onRemove: (a: AgentSummaryDto) => void;
}

export function AgentCard({
  agent,
  busy,
  runsRefreshKey,
  onRun,
  onEdit,
  onToggle,
  onRemove,
}: Props) {
  const [open, setOpen] = useState(false);

  return (
    <article
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-8)",
        padding: "var(--sp-12)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-3)",
        background: "var(--bg-raised)",
      }}
    >
      <header
        style={{
          display: "flex",
          alignItems: "baseline",
          gap: "var(--sp-8)",
          flexWrap: "wrap",
        }}
      >
        <h3
          style={{
            margin: 0,
            fontSize: "var(--fs-md)",
            color: "var(--fg)",
          }}
        >
          {agent.display_name || agent.name}
        </h3>
        <span
          style={{
            fontFamily: "var(--ff-mono)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-3)",
          }}
        >
          {agent.name}
        </span>
        <span style={{ flex: 1 }} />
        <Tag tone={agent.enabled ? "ok" : "ghost"}>
          {agent.enabled ? "enabled" : "disabled"}
        </Tag>
      </header>

      {agent.description && (
        <p
          style={{
            margin: 0,
            fontSize: "var(--fs-sm)",
            color: "var(--fg-2)",
          }}
        >
          {agent.description}
        </p>
      )}

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "max-content 1fr",
          gap: "var(--sp-4) var(--sp-12)",
          fontSize: "var(--fs-xs)",
          color: "var(--fg-2)",
        }}
      >
        <span style={{ color: "var(--fg-3)" }}>cron</span>
        <span style={{ fontFamily: "var(--ff-mono)" }}>
          {agent.cron ?? "—"}
        </span>
        <span style={{ color: "var(--fg-3)" }}>cwd</span>
        <span style={{ fontFamily: "var(--ff-mono)" }}>{agent.cwd}</span>
        <span style={{ color: "var(--fg-3)" }}>binary</span>
        <span>
          {agent.binary_kind === "first_party"
            ? "claude (first-party)"
            : `route (${agent.binary_route_id ?? "?"})`}
          {agent.model && (
            <span style={{ color: "var(--fg-3)" }}> · {agent.model}</span>
          )}
        </span>
        <span style={{ color: "var(--fg-3)" }}>permissions</span>
        <span>
          {agent.permission_mode}
          {agent.allowed_tools.length > 0 && (
            <span
              style={{
                color: "var(--fg-3)",
                fontFamily: "var(--ff-mono)",
                marginLeft: "var(--sp-4)",
              }}
            >
              [{agent.allowed_tools.join(", ")}]
            </span>
          )}
        </span>
        {agent.max_budget_usd !== null && (
          <>
            <span style={{ color: "var(--fg-3)" }}>budget</span>
            <span>${agent.max_budget_usd.toFixed(2)}</span>
          </>
        )}
      </div>

      <footer
        style={{
          display: "flex",
          gap: "var(--sp-6)",
          flexWrap: "wrap",
        }}
      >
        <Button
          variant="solid"
          onClick={() => onRun(agent.id)}
          disabled={busy}
        >
          Run now
        </Button>
        <Button
          variant="ghost"
          onClick={() => onEdit(agent)}
          disabled={busy}
        >
          Edit
        </Button>
        <Button
          variant="ghost"
          onClick={() => onToggle(agent.id, !agent.enabled)}
          disabled={busy}
        >
          {agent.enabled ? "Disable" : "Enable"}
        </Button>
        <Button
          variant="ghost"
          onClick={() => onRemove(agent)}
          disabled={busy}
        >
          Delete
        </Button>
        <span style={{ flex: 1 }} />
        <Button variant="ghost" onClick={() => setOpen((o) => !o)}>
          {open ? "Hide runs" : "Show runs"}
        </Button>
      </footer>

      {open && (
        <RunHistoryPanel
          agentId={agent.id}
          refreshKey={runsRefreshKey}
        />
      )}
    </article>
  );
}
