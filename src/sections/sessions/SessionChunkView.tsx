import { type CSSProperties, useState } from "react";
import type {
  LinkedTool,
  SessionChunk,
  SessionEvent,
} from "../../types";
import { Glyph } from "../../components/primitives/Glyph";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";
import { ToolExecutionView } from "./viewers";
import { redactSecrets } from "./viewers/redact";
import { formatTokens, modelBadge } from "./format";
import { stripLocalCommandStdout } from "./localCommandStdout";

const TEXT_CLAMP = 4000;

/**
 * Paper-mono renderer for one `SessionChunk`.
 *
 * Chunks are the unit of composition here — the old per-event renderer
 * still exists in `SessionEventView.tsx`, but it's noisy for long
 * transcripts. Chunking folds noise down and lets the AI chunk's tool
 * executions render inside a single bubble through the
 * `ToolExecutionView` dispatcher.
 */
export function SessionChunkView({
  chunk,
  events,
  searchTerm,
}: {
  chunk: SessionChunk;
  events: SessionEvent[];
  searchTerm: string;
}) {
  switch (chunk.chunkType) {
    case "user": {
      const ev = events[chunk.event_index];
      const text = ev && "text" in ev && typeof ev.text === "string" ? ev.text : "";
      const ts = ev && "ts" in ev ? ev.ts : null;
      return (
        <Bubble side="left" tone="sunken">
          {renderHeader("You", ts)}
          <Body text={redactSecrets(text)} searchTerm={searchTerm} />
        </Bubble>
      );
    }
    case "system": {
      const ev = events[chunk.event_index];
      const raw = ev && "text" in ev && typeof ev.text === "string" ? ev.text : "";
      const text = redactSecrets(stripLocalCommandStdout(raw));
      const ts = ev && "ts" in ev ? ev.ts : null;
      return (
        <Bubble side="left" tone="faint" mono>
          {renderHeader("System output", ts)}
          <Body text={text} searchTerm={searchTerm} mono tone="ghost" />
        </Bubble>
      );
    }
    case "compact": {
      const ev = events[chunk.event_index];
      const text = ev && "text" in ev && typeof ev.text === "string" ? ev.text : "";
      return (
        <Divider>
          <Tag tone="accent" glyph={NF.archive}>
            Compacted
          </Tag>
          <Body text={redactSecrets(text)} searchTerm={searchTerm} tone="ghost" />
        </Divider>
      );
    }
    case "ai":
      return (
        <AiChunkView
          chunk={chunk}
          events={events}
          searchTerm={searchTerm}
        />
      );
  }
}

function AiChunkView({
  chunk,
  events,
  searchTerm,
}: {
  chunk: Extract<SessionChunk, { chunkType: "ai" }>;
  events: SessionEvent[];
  searchTerm: string;
}) {
  const toolsByCallIdx = new Map<number, LinkedTool>(
    chunk.tool_executions.map((t) => [t.call_index, t]),
  );
  const absorbed = new Set<number>(
    chunk.tool_executions
      .map((t) => t.result_index)
      .filter((i): i is number => i != null),
  );
  const usageBits: string[] = [];
  const tokens = chunk.metrics.tokens;
  const tokenTotal =
    tokens.input + tokens.output + tokens.cache_creation + tokens.cache_read;
  if (tokenTotal > 0) usageBits.push(`${formatTokens(tokenTotal)} tok`);
  if (chunk.metrics.tool_call_count > 0) {
    usageBits.push(
      `${chunk.metrics.tool_call_count} tool${chunk.metrics.tool_call_count === 1 ? "" : "s"}`,
    );
  }
  if (chunk.metrics.thinking_count > 0) {
    usageBits.push(
      `${chunk.metrics.thinking_count} think${chunk.metrics.thinking_count === 1 ? "" : "s"}`,
    );
  }

  return (
    <Bubble side="right" tone="accent">
      {renderHeader(
        "Claude",
        chunk.start_ts,
        usageBits.length > 0 ? usageBits.join(" · ") : null,
      )}
      <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)" }}>
        {chunk.event_indices.map((idx) => {
          if (absorbed.has(idx)) return null;
          const ev = events[idx];
          if (!ev) return null;
          const tool = toolsByCallIdx.get(idx);
          if (tool) {
            return <ToolExecutionView key={idx} tool={tool} />;
          }
          return (
            <EventInlineView key={idx} event={ev} searchTerm={searchTerm} />
          );
        })}
      </div>
    </Bubble>
  );
}

function EventInlineView({
  event,
  searchTerm,
}: {
  event: SessionEvent;
  searchTerm: string;
}) {
  switch (event.kind) {
    case "assistantText": {
      const model = event.model;
      return (
        <div>
          {model && (
            <div
              style={{
                fontSize: "var(--fs-3xs)",
                color: "var(--fg-faint)",
                letterSpacing: "var(--ls-wide)",
                textTransform: "uppercase",
                marginBottom: "var(--sp-2)",
              }}
            >
              {modelBadge([model])}
            </div>
          )}
          <Body text={redactSecrets(event.text)} searchTerm={searchTerm} />
        </div>
      );
    }
    case "assistantThinking":
      return (
        <details
          style={{
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-2)",
            padding: "var(--sp-4) var(--sp-8)",
          }}
        >
          <summary
            style={{
              cursor: "pointer",
              fontSize: "var(--fs-xs)",
              color: "var(--fg-muted)",
              textTransform: "uppercase",
              letterSpacing: "var(--ls-wide)",
            }}
          >
            <Glyph g={NF.bolt} style={{ fontSize: "var(--fs-2xs)" }} /> Thinking
          </summary>
          <div style={{ marginTop: "var(--sp-4)" }}>
            <Body text={redactSecrets(event.text)} searchTerm={searchTerm} tone="ghost" />
          </div>
        </details>
      );
    case "assistantToolUse":
      // Orphaned tool call — the linker didn't find a matching result.
      return (
        <div
          style={{
            padding: "var(--sp-6) var(--sp-10)",
            fontSize: "var(--fs-xs)",
            border: "var(--bw-hair) dashed var(--line)",
            borderRadius: "var(--r-2)",
            color: "var(--fg-muted)",
          }}
        >
          🔧 {redactSecrets(event.tool_name)} <span className="mono">{event.tool_use_id.slice(0, 8)}</span> · (no result)
        </div>
      );
    case "userToolResult":
      // Result without a linked call — surface so we don't lose it.
      return (
        <div
          className="mono"
          style={{
            padding: "var(--sp-6) var(--sp-10)",
            fontSize: "var(--fs-xs)",
            border: "var(--bw-hair) dashed var(--line)",
            borderRadius: "var(--r-2)",
            color: event.is_error ? "var(--warn)" : "var(--fg-muted)",
          }}
        >
          {event.is_error ? "⚠ " : "↩ "}
          {redactSecrets(event.content.slice(0, 400))}
        </div>
      );
    case "malformed":
      return (
        <div style={{ color: "var(--warn)", fontSize: "var(--fs-xs)" }}>
          Malformed line {event.line_number}: {redactSecrets(event.error)}
        </div>
      );
    case "attachment":
      return (
        <div
          style={{
            padding: "var(--sp-6) var(--sp-10)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-muted)",
            border: "var(--bw-hair) dashed var(--line)",
            borderRadius: "var(--r-2)",
          }}
        >
          📎 Attachment {event.name ? redactSecrets(event.name) : "(unnamed)"}
          {event.mime ? ` · ${redactSecrets(event.mime)}` : ""}
        </div>
      );
    case "other":
      return (
        <div
          style={{
            padding: "var(--sp-4) var(--sp-10)",
            fontSize: "var(--fs-3xs)",
            color: "var(--fg-ghost)",
            letterSpacing: "var(--ls-wide)",
            textTransform: "uppercase",
          }}
        >
          {redactSecrets(event.raw_type)}
        </div>
      );
    default:
      // Exhaustiveness guard: make TypeScript flag any future variant
      // that we forget to render inside an AI chunk instead of silently
      // hiding it.
      return null;
  }
}

// ---------------------------------------------------------------------------
// Atoms (kept locally to avoid touching SessionEventView.tsx which is still
// used as a fallback when chunks fail to load).
// ---------------------------------------------------------------------------

type BubbleTone = "sunken" | "accent" | "faint" | "ghost";

function Bubble({
  side,
  tone,
  mono,
  children,
}: {
  side: "left" | "right";
  tone: BubbleTone;
  mono?: boolean;
  children: React.ReactNode;
}) {
  const palette: Record<BubbleTone, { bg: string; bd: string }> = {
    sunken: { bg: "var(--bg-sunken)", bd: "var(--line)" },
    accent: { bg: "var(--accent-soft)", bd: "var(--accent-border)" },
    faint: { bg: "var(--bg-raised)", bd: "var(--line)" },
    ghost: { bg: "transparent", bd: "var(--line)" },
  };
  const p = palette[tone];
  return (
    <div
      style={{
        display: "flex",
        justifyContent: side === "right" ? "flex-end" : "flex-start",
        marginBottom: "var(--sp-10)",
      }}
    >
      <div
        style={{
          maxWidth: "min(var(--content-cap-lg), 92%)",
          minWidth: "min(280px, 60%)",
          padding: "var(--sp-10) var(--sp-14)",
          background: p.bg,
          border: `var(--bw-hair) solid ${p.bd}`,
          borderRadius: "var(--r-2)",
          fontFamily: mono ? "var(--font)" : undefined,
          fontSize: "var(--fs-sm)",
          color: "var(--fg)",
          whiteSpace: "pre-wrap",
          wordBreak: "break-word",
        }}
      >
        {children}
      </div>
    </div>
  );
}

function Divider({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-10)",
        margin: "var(--sp-14) 0",
      }}
    >
      <div style={{ flex: 1, height: 1, background: "var(--line)" }} />
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-6)",
          color: "var(--fg-faint)",
          fontSize: "var(--fs-xs)",
        }}
      >
        {children}
      </div>
      <div style={{ flex: 1, height: 1, background: "var(--line)" }} />
    </div>
  );
}

function renderHeader(
  label: string,
  ts: string | null | undefined,
  extra?: string | null,
) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-8)",
        marginBottom: "var(--sp-6)",
        fontSize: "var(--fs-xs)",
        color: "var(--fg-faint)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
      }}
    >
      <span>{label}</span>
      {ts && <span title={ts}>{new Date(ts).toLocaleString()}</span>}
      {extra && <span style={{ marginLeft: "auto" }}>{extra}</span>}
    </div>
  );
}

function Body({
  text,
  searchTerm,
  mono,
  tone,
}: {
  text: string;
  searchTerm: string;
  mono?: boolean;
  tone?: "ghost" | "warn";
}) {
  const [expanded, setExpanded] = useState(false);
  const trimmed = text ?? "";
  const overflow = trimmed.length > TEXT_CLAMP;
  const visible =
    expanded || !overflow ? trimmed : trimmed.slice(0, TEXT_CLAMP);
  const baseStyle: CSSProperties = {
    fontFamily: mono ? "var(--font)" : undefined,
    fontSize: mono ? "var(--fs-xs)" : "var(--fs-sm)",
    color:
      tone === "warn"
        ? "var(--warn)"
        : tone === "ghost"
          ? "var(--fg-muted)"
          : "var(--fg)",
    whiteSpace: "pre-wrap",
    wordBreak: "break-word",
  };
  return (
    <>
      <div style={baseStyle}>{highlight(visible, searchTerm)}</div>
      {overflow && (
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          style={{
            marginTop: "var(--sp-4)",
            background: "transparent",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-1)",
            color: "var(--fg-muted)",
            fontSize: "var(--fs-xs)",
            padding: "var(--sp-2) var(--sp-8)",
            cursor: "pointer",
            letterSpacing: "var(--ls-wide)",
            textTransform: "uppercase",
          }}
        >
          {expanded
            ? "Collapse"
            : `Show ${trimmed.length - TEXT_CLAMP} more chars`}
        </button>
      )}
    </>
  );
}

function highlight(text: string, term: string): React.ReactNode {
  if (!term || term.length < 2) return text;
  try {
    const pattern = new RegExp(escapeRegex(term), "gi");
    const parts: React.ReactNode[] = [];
    let lastIdx = 0;
    let match: RegExpExecArray | null;
    let key = 0;
    while ((match = pattern.exec(text)) !== null) {
      if (match.index > lastIdx) parts.push(text.slice(lastIdx, match.index));
      parts.push(
        <mark
          key={`h${key++}`}
          style={{
            background: "var(--accent-soft)",
            color: "var(--accent-ink)",
          }}
        >
          {match[0]}
        </mark>,
      );
      lastIdx = match.index + match[0].length;
      if (match.index === pattern.lastIndex) pattern.lastIndex += 1;
    }
    if (lastIdx < text.length) parts.push(text.slice(lastIdx));
    return parts;
  } catch {
    return text;
  }
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
