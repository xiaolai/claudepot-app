import { useState } from "react";
import { Button } from "../../components/primitives/Button";
import { Tag } from "../../components/primitives/Tag";
import type { AutomationSummaryDto } from "../../types";
import { RunHistoryPanel } from "./RunHistoryPanel";

interface Props {
  automation: AutomationSummaryDto;
  busy: boolean;
  /** Increments when a run completes — RunHistoryPanel re-fetches. */
  runsRefreshKey: number;
  onRun: (id: string) => void;
  onEdit: (a: AutomationSummaryDto) => void;
  onToggle: (id: string, enabled: boolean) => void;
  onRemove: (a: AutomationSummaryDto) => void;
}

export function AutomationCard({
  automation,
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
          {automation.display_name || automation.name}
        </h3>
        <span
          style={{
            fontFamily: "var(--ff-mono)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-3)",
          }}
        >
          {automation.name}
        </span>
        <span style={{ flex: 1 }} />
        <Tag tone={automation.enabled ? "ok" : "ghost"}>
          {automation.enabled ? "enabled" : "disabled"}
        </Tag>
      </header>

      {automation.description && (
        <p
          style={{
            margin: 0,
            fontSize: "var(--fs-sm)",
            color: "var(--fg-2)",
          }}
        >
          {automation.description}
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
          {automation.cron ?? "—"}
        </span>
        <span style={{ color: "var(--fg-3)" }}>cwd</span>
        <span style={{ fontFamily: "var(--ff-mono)" }}>{automation.cwd}</span>
        <span style={{ color: "var(--fg-3)" }}>binary</span>
        <span>
          {automation.binary_kind === "first_party"
            ? "claude (first-party)"
            : `route (${automation.binary_route_id ?? "?"})`}
          {automation.model && (
            <span style={{ color: "var(--fg-3)" }}> · {automation.model}</span>
          )}
        </span>
        <span style={{ color: "var(--fg-3)" }}>permissions</span>
        <span>
          {automation.permission_mode}
          {automation.allowed_tools.length > 0 && (
            <span
              style={{
                color: "var(--fg-3)",
                fontFamily: "var(--ff-mono)",
                marginLeft: "var(--sp-4)",
              }}
            >
              [{automation.allowed_tools.join(", ")}]
            </span>
          )}
        </span>
        {automation.max_budget_usd !== null && (
          <>
            <span style={{ color: "var(--fg-3)" }}>budget</span>
            <span>${automation.max_budget_usd.toFixed(2)}</span>
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
          onClick={() => onRun(automation.id)}
          disabled={busy}
        >
          Run now
        </Button>
        <Button
          variant="ghost"
          onClick={() => onEdit(automation)}
          disabled={busy}
        >
          Edit
        </Button>
        <Button
          variant="ghost"
          onClick={() => onToggle(automation.id, !automation.enabled)}
          disabled={busy}
        >
          {automation.enabled ? "Disable" : "Enable"}
        </Button>
        <Button
          variant="ghost"
          onClick={() => onRemove(automation)}
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
          automationId={automation.id}
          refreshKey={runsRefreshKey}
        />
      )}
    </article>
  );
}
