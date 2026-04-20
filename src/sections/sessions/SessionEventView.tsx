import { type CSSProperties, useState } from "react";
import type { NfIcon } from "../../icons";
import { Glyph } from "../../components/primitives/Glyph";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";
import type { SessionEvent } from "../../types";
import { formatTokens, modelBadge } from "./format";

const MESSAGE_CLAMP = 4000;
const CODE_CLAMP = 4000;

/**
 * Paper-mono transcript event bubble. Preserves CC's role / tool-use
 * geometry so a reader can reconstruct the turn flow: user messages
 * pin left on a sunken surface, assistant text pins against the
 * accent-tinted surface, and tool traffic sits in a narrow mono
 * callout between them.
 *
 * Long text is clamped with "Show more" so a 300-turn session doesn't
 * blow out the render pass. The clamp is per-event, not per-section,
 * so expanding one block doesn't reflow the rest.
 */
export function SessionEventView({
  event,
  searchTerm,
}: {
  event: SessionEvent;
  searchTerm: string;
}) {
  const ts = event.kind === "malformed" ? null : event.ts;

  const header = (label: string, extra?: React.ReactNode) => (
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

  switch (event.kind) {
    case "userText":
      return (
        <Bubble side="left" tone="sunken">
          {header("You")}
          <Body text={event.text} searchTerm={searchTerm} />
        </Bubble>
      );

    case "userToolResult":
      return (
        <Bubble side="left" tone="faint" mono>
          {header(
            event.is_error ? "Tool result · error" : "Tool result",
            <span className="mono" style={{ color: "var(--fg-ghost)" }}>
              {event.tool_use_id.slice(0, 8)}
            </span>,
          )}
          <Body
            text={event.content}
            searchTerm={searchTerm}
            mono
            clamp={CODE_CLAMP}
            tone={event.is_error ? "warn" : undefined}
          />
        </Bubble>
      );

    case "assistantText": {
      const usageBits: string[] = [];
      if (event.usage) {
        const t = event.usage.total;
        if (t > 0) usageBits.push(`${formatTokens(t)} tok`);
      }
      if (event.stop_reason && event.stop_reason !== "end_turn") {
        usageBits.push(event.stop_reason);
      }
      return (
        <Bubble side="right" tone="accent">
          {header(
            "Claude",
            <span
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: "var(--sp-6)",
              }}
            >
              {event.model && (
                <span style={{ color: "var(--fg-muted)" }}>
                  {modelBadge([event.model])}
                </span>
              )}
              {usageBits.length > 0 && (
                <span style={{ color: "var(--fg-faint)" }}>
                  {usageBits.join(" · ")}
                </span>
              )}
            </span>,
          )}
          <Body text={event.text} searchTerm={searchTerm} />
        </Bubble>
      );
    }

    case "assistantToolUse":
      return (
        <Bubble side="right" tone="faint" mono>
          {header(
            `Tool call · ${event.tool_name}`,
            <span className="mono" style={{ color: "var(--fg-ghost)" }}>
              {event.tool_use_id.slice(0, 8)}
            </span>,
          )}
          <Body
            text={event.input_preview}
            searchTerm={searchTerm}
            mono
            clamp={CODE_CLAMP}
          />
        </Bubble>
      );

    case "assistantThinking":
      return (
        <Bubble side="right" tone="ghost">
          {header("Thinking")}
          <Body text={event.text} searchTerm={searchTerm} tone="ghost" />
        </Bubble>
      );

    case "summary":
      return (
        <Divider>
          <Tag tone="accent" glyph={NF.archive}>
            Compacted
          </Tag>
          <Body text={event.text} searchTerm={searchTerm} tone="ghost" />
        </Divider>
      );

    case "system":
      return (
        <MiniLine glyph={NF.info} tone="ghost">
          {event.subtype ?? "system"} · {event.detail || "(no detail)"}
        </MiniLine>
      );

    case "attachment":
      return (
        <MiniLine glyph={NF.file} tone="muted">
          Attachment {event.name ?? "(unnamed)"}
          {event.mime ? ` · ${event.mime}` : ""}
        </MiniLine>
      );

    case "fileSnapshot":
      return (
        <MiniLine glyph={NF.archive} tone="ghost">
          File-history snapshot · {event.file_count} file
          {event.file_count === 1 ? "" : "s"}
        </MiniLine>
      );

    case "other":
      return (
        <MiniLine glyph={NF.circle} tone="ghost">
          {event.raw_type}
        </MiniLine>
      );

    case "malformed":
      return (
        <MiniLine glyph={NF.warn} tone="warn">
          Malformed JSONL line {event.line_number}: {event.error}
        </MiniLine>
      );
  }
}

// ---------------------------------------------------------------------------
// Atoms
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
    sunken: {
      bg: "var(--bg-sunken)",
      bd: "var(--line)",
    },
    accent: {
      bg: "var(--accent-soft)",
      bd: "var(--accent-border)",
    },
    faint: {
      bg: "var(--bg-raised)",
      bd: "var(--line)",
    },
    ghost: {
      bg: "transparent",
      bd: "var(--line)",
    },
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

function MiniLine({
  glyph,
  tone,
  children,
}: {
  glyph: NfIcon;
  tone: "muted" | "ghost" | "warn";
  children: React.ReactNode;
}) {
  const color =
    tone === "warn"
      ? "var(--warn)"
      : tone === "muted"
        ? "var(--fg-muted)"
        : "var(--fg-ghost)";
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-6)",
        padding: "var(--sp-4) var(--sp-12)",
        margin: "var(--sp-4) 0",
        color,
        fontSize: "var(--fs-xs)",
      }}
    >
      <Glyph g={glyph} style={{ fontSize: "var(--fs-2xs)" }} />
      <span>{children}</span>
    </div>
  );
}

function Body({
  text,
  searchTerm,
  mono,
  clamp = MESSAGE_CLAMP,
  tone,
}: {
  text: string;
  searchTerm: string;
  mono?: boolean;
  clamp?: number;
  tone?: "ghost" | "warn";
}) {
  const [expanded, setExpanded] = useState(false);
  const trimmed = text ?? "";
  const overflow = trimmed.length > clamp;
  const visible = expanded || !overflow ? trimmed : trimmed.slice(0, clamp);

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
            : `Show ${trimmed.length - clamp} more chars`}
        </button>
      )}
    </>
  );
}

/**
 * Case-insensitive highlight of the search term. Chunks the source
 * string on the search match, wrapping hits in a `<mark>` with the
 * accent-soft background. Short-circuits on empty queries so we don't
 * pay the regex cost per event on every keystroke of the filter box.
 */
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
