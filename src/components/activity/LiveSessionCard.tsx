// Live session card (WI-L3 of sessions-live.md).
//
// One card per active session. Rendered as a grid item in the
// LiveSessionsStrip at the top of the Activities tab (WI-L4).
//
// Motion vocabulary follows the paper-mono register:
//   - Active dot pulses when status === "busy".
//   - All other state is static.
//   - prefers-reduced-motion drops the pulse, keeps the dot
//     visible as a state badge.
//
// Source-kind badge (Claude / Codex) renders via the
// `source_kind` field on LiveSessionSummary. The current runtime
// only emits ClaudeCode summaries; Codex sessions will appear
// here automatically when the runtime is extended (WI-L1/L2).

import type { LiveSessionSummary } from "../../types";
import { NF } from "../../icons";
import { basename } from "../../lib/paths";
import { Tag } from "../primitives/Tag";

interface Props {
  summary: LiveSessionSummary;
  onClick?: () => void;
}

export function LiveSessionCard({ summary, onClick }: Props) {
  const sourceLabel =
    (summary as LiveSessionSummary & { source_kind?: string }).source_kind ===
    "codex"
      ? "Codex"
      : "Claude";
  const busy = summary.status === "busy";
  const project = projectFromCwd(summary.cwd);
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        textAlign: "left",
        background: "var(--bg-raised)",
        border: "var(--sp-px) solid var(--line)",
        borderRadius: "var(--r-3)",
        padding: "var(--sp-12)",
        font: "inherit",
        cursor: onClick ? "pointer" : "default",
        display: "flex",
        flexDirection: "column",
        gap: 8,
        minWidth: 0,
      }}
      aria-label={`Live session ${summary.session_id} in ${project}, status ${summary.status}`}
    >
      <header style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <span
          aria-hidden="true"
          className={busy ? "live-dot-busy" : "live-dot-idle"}
          style={{
            width: "var(--sp-8)",
            height: "var(--sp-8)",
            borderRadius: "var(--r-3)",
            background: busy ? "var(--accent)" : "var(--fg-muted)",
            flex: "0 0 var(--sp-8)",
          }}
        />
        <Tag>{sourceLabel}</Tag>
        <Tag>{summary.status}</Tag>
        <div style={{ flex: 1 }} />
        {/* Lucide glyphs, not emoji — design.md bans emoji icons
            (⚠/⏳ render as color emoji in WebKit, breaking the
            monochrome register). Glyph + text keeps the "color
            never alone" rule satisfied. */}
        {summary.errored && <Tag glyph={NF.warn}>error</Tag>}
        {summary.stuck && <Tag glyph={NF.hourglass}>stuck</Tag>}
      </header>
      <div
        style={{
          fontSize: "var(--fs-sm)",
          fontWeight: 500,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
        title={project}
      >
        {project}
      </div>
      {summary.current_action && (
        <div
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-muted)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            display: "-webkit-box",
            WebkitLineClamp: 2,
            WebkitBoxOrient: "vertical" as const,
          }}
        >
          {summary.current_action}
        </div>
      )}
      <footer
        style={{
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-muted)",
          display: "flex",
          gap: 8,
        }}
      >
        {summary.model && <span title="Model">{summary.model}</span>}
        {summary.idle_ms > 0 && (
          <span style={{ marginLeft: "auto" }} title="Time since last delta">
            {humanizeMs(summary.idle_ms)}
          </span>
        )}
      </footer>
    </button>
  );
}

function projectFromCwd(cwd: string): string {
  // Windows-aware via lib/paths (audit 2026-07 F2).
  return basename(cwd);
}

function humanizeMs(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  return `${h}h`;
}
