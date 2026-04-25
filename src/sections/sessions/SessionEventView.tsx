import { useState } from "react";
import type { NfIcon } from "../../icons";
import { Glyph } from "../../components/primitives/Glyph";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";
import type { SessionEvent } from "../../types";
import { useActivityPrefs } from "../../hooks/useActivityPrefs";
import { Body, Bubble, Divider } from "./components/transcriptAtoms";
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
  const prefs = useActivityPrefs();

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
          <Body text={event.text} searchTerm={searchTerm} clamp={MESSAGE_CLAMP} />
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
          <Body text={event.text} searchTerm={searchTerm} clamp={MESSAGE_CLAMP} />
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
          <ThinkingBody
            text={event.text}
            searchTerm={searchTerm}
            hideByDefault={prefs.hideThinking}
          />
        </Bubble>
      );

    case "summary":
      return (
        <Divider>
          <Tag tone="accent" glyph={NF.archive}>
            Compacted
          </Tag>
          <Body
            text={event.text}
            searchTerm={searchTerm}
            clamp={MESSAGE_CLAMP}
            tone="ghost"
          />
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

    case "taskSummary":
      // Boundary marker emitted after /compact or when an agent
      // finalizes a run. Render the summary inline so users can
      // see the compaction reason without expanding the event.
      // Re-uses the same `summary` glyph as the existing `summary`
      // kind above — both are end-of-arc markers visually.
      return (
        <MiniLine glyph={NF.book} tone="ghost">
          Task summary · {event.summary}
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
// Atoms (Bubble / Divider / Body / highlight / escapeRegex live in the
// shared transcriptAtoms module — both transcript renderers consume
// them. MiniLine is local because only this view needs it.)
// ---------------------------------------------------------------------------

/**
 * Renders an assistantThinking block. When `hideByDefault` is true
 * (driven by the `activity_hide_thinking` pref) the body is replaced
 * with "Thinking · N chars — click to reveal" until the user clicks.
 *
 * State semantics:
 *   - `revealedByUser` latches once the user explicitly clicks to
 *     reveal. A subsequent pref flip back to hide leaves their
 *     expanded view alone.
 *   - While `revealedByUser` is still false, the rendered state
 *     follows the pref directly — so flipping Hide Thinking on in
 *     Settings immediately collapses previously-open blocks.
 *
 * This is cleaner than the earlier revealed-state-copy-of-pref
 * approach, which stuck on whichever value the pref held at first
 * mount.
 */
function ThinkingBody({
  text,
  searchTerm,
  hideByDefault,
}: {
  text: string;
  searchTerm: string;
  hideByDefault: boolean;
}) {
  const [revealedByUser, setRevealedByUser] = useState(false);
  const shown = revealedByUser || !hideByDefault;

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
    <Body
      text={text}
      searchTerm={searchTerm}
      clamp={MESSAGE_CLAMP}
      tone="ghost"
    />
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

