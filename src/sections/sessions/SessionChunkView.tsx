import { useState } from "react";
import type {
  LinkedTool,
  SessionChunk,
  SessionEvent,
} from "../../types";
import { Glyph } from "../../components/primitives/Glyph";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";
import { useActivityPrefs } from "../../hooks/useActivityPrefs";
import { Body, Divider, FoldableBubble } from "./components/transcriptAtoms";
import { ToolExecutionView } from "./viewers";
import { redactSecrets } from "../../lib/redactSecrets";
import { formatTokens, modelBadge } from "./format";
import { stripLocalCommandStdout } from "./localCommandStdout";
import { CopyButton } from "../../components/CopyButton";

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
      const copyText = redactSecrets(text);
      return (
        <FoldableBubble
          side="left"
          tone="sunken"
          foldText={copyText}
          searchTerm={searchTerm}
          header={renderHeader(
            "You",
            ts,
            <CopyButton text={copyText} ariaLabel="Copy your message" />,
          )}
        >
          <Body
            text={copyText}
            searchTerm={searchTerm}
            clamp={TEXT_CLAMP}
          />
        </FoldableBubble>
      );
    }
    case "system": {
      const ev = events[chunk.event_index];
      const raw = ev && "text" in ev && typeof ev.text === "string" ? ev.text : "";
      const text = redactSecrets(stripLocalCommandStdout(raw));
      const ts = ev && "ts" in ev ? ev.ts : null;
      return (
        <FoldableBubble
          side="left"
          tone="faint"
          mono
          foldText={text}
          searchTerm={searchTerm}
          header={renderHeader(
            "System output",
            ts,
            <CopyButton text={text} ariaLabel="Copy system output" />,
          )}
        >
          <Body
            text={text}
            searchTerm={searchTerm}
            clamp={TEXT_CLAMP}
            mono
            tone="ghost"
          />
        </FoldableBubble>
      );
    }
    case "compact": {
      const ev = events[chunk.event_index];
      const text = ev && "text" in ev && typeof ev.text === "string" ? ev.text : "";
      const copyText = redactSecrets(text);
      return (
        <Divider>
          <Tag tone="accent" glyph={NF.archive}>
            Compacted
          </Tag>
          <CopyButton text={copyText} ariaLabel="Copy compaction summary" />
          <Body
            text={copyText}
            searchTerm={searchTerm}
            clamp={TEXT_CLAMP}
            tone="ghost"
          />
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
  // Concatenated text payload for the chunk-level copy button. We
  // include assistant text + thinking (in order) — tool calls have
  // their own per-card copy buttons further down, so duplicating
  // their input/result here would bloat the clipboard for marginal
  // benefit. Empty when the chunk is pure tool-calls.
  const copyText = chunk.event_indices
    .map((idx) => {
      const ev = events[idx];
      if (!ev) return "";
      if (ev.kind === "assistantText") return redactSecrets(ev.text);
      if (ev.kind === "assistantThinking")
        return `[thinking]\n${redactSecrets(ev.text)}`;
      return "";
    })
    .filter((s) => s.length > 0)
    .join("\n\n");

  return (
    // `foldText` is the turn's prose (assistant text + thinking), not its
    // tool cards — a tool-only turn stays unfolded because the header
    // already reports the tool count and the cards carry their own
    // disclosure. This is the surface the "long answers" fold targets.
    <FoldableBubble
      side="right"
      tone="accent"
      foldText={copyText}
      searchTerm={searchTerm}
      header={renderHeader(
        "Claude",
        chunk.start_ts,
        <>
          {usageBits.length > 0 && (
            <span style={{ color: "var(--fg-faint)" }}>
              {usageBits.join(" · ")}
            </span>
          )}
          {copyText.length > 0 && (
            <CopyButton text={copyText} ariaLabel="Copy Claude's turn" />
          )}
        </>,
      )}
    >
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
    </FoldableBubble>
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
          <Body
            text={redactSecrets(event.text)}
            searchTerm={searchTerm}
            clamp={TEXT_CLAMP}
          />
        </div>
      );
    }
    case "assistantThinking":
      return <ThinkingDetails text={event.text} searchTerm={searchTerm} />;
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
          <Glyph g={NF.wrench} /> {redactSecrets(event.tool_name)} <span className="mono">{event.tool_use_id.slice(0, 8)}</span> · (no result)
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
          {redactSecrets(event.content).slice(0, 400)}
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
          <Glyph g={NF.paperclip} /> Attachment {event.name ? redactSecrets(event.name) : "(unnamed)"}
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

function renderHeader(
  label: string,
  ts: string | null | undefined,
  extra?: React.ReactNode,
) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        // Wrap as whole items. Without this the label and timestamp are
        // shrinkable flex items and a tight bubble breaks them
        // mid-word ("CLAUD / E") — visible once the fold chevron takes
        // its ~28px out of the header row.
        flexWrap: "wrap",
        gap: "var(--sp-8)",
        marginBottom: "var(--sp-6)",
        fontSize: "var(--fs-xs)",
        color: "var(--fg-faint)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
      }}
    >
      <span style={{ whiteSpace: "nowrap" }}>{label}</span>
      {ts && (
        <span title={ts} style={{ whiteSpace: "nowrap" }}>
          {new Date(ts).toLocaleString()}
        </span>
      )}
      {extra && (
        <span
          style={{
            marginLeft: "auto",
            display: "inline-flex",
            alignItems: "center",
            gap: "var(--sp-6)",
          }}
        >
          {extra}
        </span>
      )}
    </div>
  );
}

/**
 * Chunks-mode renderer for an assistantThinking event. Mirrors
 * ThinkingBody in SessionEventView — honors the activity_hide_thinking
 * preference and latches a user reveal. Chunks mode is the default
 * view, so wiring this here is what makes the pref feel real.
 *
 * Exported for unit tests; the chunk dispatcher above is the only
 * production consumer.
 */
export function ThinkingDetails({
  text,
  searchTerm,
}: {
  text: string;
  searchTerm: string;
}) {
  const { hideThinking } = useActivityPrefs();
  const [revealedByUser, setRevealedByUser] = useState(false);
  const shown = revealedByUser || !hideThinking;
  if (!shown) {
    return (
      <button
        type="button"
        onClick={() => setRevealedByUser(true)}
        className="pm-focus"
        style={{
          alignSelf: "flex-start",
          display: "inline-flex",
          alignItems: "center",
          gap: "var(--sp-6)",
          padding: "var(--sp-4) var(--sp-10)",
          fontSize: "var(--fs-xs)",
          fontStyle: "italic",
          color: "var(--fg-faint)",
          background: "var(--bg-sunken)",
          border: "var(--bw-hair) dashed var(--line)",
          borderRadius: "var(--r-2)",
          cursor: "pointer",
          fontFamily: "var(--font)",
        }}
        aria-label={`Reveal thinking block (${text.length} characters)`}
      >
        Thinking · {text.length.toLocaleString()} chars — click to reveal
      </button>
    );
  }
  return (
    <details
      open
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
        Thinking
      </summary>
      <div style={{ marginTop: "var(--sp-4)" }}>
        <Body
          text={redactSecrets(text)}
          searchTerm={searchTerm}
          clamp={TEXT_CLAMP}
          tone="ghost"
        />
      </div>
    </details>
  );
}

