import type { LinkedTool } from "../../../types";
import { Glyph } from "../../../components/primitives/Glyph";
import { NF } from "../../../icons";
import { parseToolInput, tryParseResult, type BashInput, type BashResult } from "./toolInput";

const OUTPUT_CLAMP = 4000;

/**
 * Bash tool viewer — renders the command, optional description/timeout,
 * and separates stdout from stderr when the result body is JSON. Plain
 * stdout bodies render into a single block.
 */
export function BashToolViewer({ tool }: { tool: LinkedTool }) {
  const parsed = parseToolInput<BashInput>(tool.input_preview);
  const input = parsed.ok ? parsed.value : {};
  const resultRaw = tool.result_content ?? "";
  const result = tryParseResult<BashResult>(resultRaw);

  const stdout = result.ok ? (result.value.stdout ?? "") : resultRaw;
  const stderr = result.ok ? (result.value.stderr ?? "") : "";
  const exit = result.ok ? result.value.exit_code : undefined;
  const interrupted = result.ok ? result.value.interrupted : undefined;

  return (
    <div
      data-testid="bash-tool-viewer"
      style={{
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        background: "var(--bg-raised)",
      }}
    >
      <header
        style={{
          padding: "var(--sp-6) var(--sp-10)",
          borderBottom: "var(--bw-hair) solid var(--line)",
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
          fontSize: "var(--fs-xs)",
          color: "var(--fg-muted)",
        }}
      >
        <Glyph g={NF.terminal} style={{ fontSize: "var(--fs-sm)" }} />
        <span className="mono" style={{ flex: 1 }} title={input.command ?? ""}>
          $ {input.command ?? "(no command)"}
        </span>
        {typeof exit === "number" && (
          <span
            style={{
              color: exit === 0 ? "var(--ok)" : "var(--warn)",
              fontSize: "var(--fs-3xs)",
              letterSpacing: "var(--ls-wide)",
              textTransform: "uppercase",
            }}
          >
            exit {exit}
          </span>
        )}
        {interrupted && (
          <span
            style={{
              color: "var(--warn)",
              fontSize: "var(--fs-3xs)",
              letterSpacing: "var(--ls-wide)",
              textTransform: "uppercase",
            }}
          >
            interrupted
          </span>
        )}
        {tool.is_error && (
          <span
            style={{
              color: "var(--warn)",
              textTransform: "uppercase",
              fontSize: "var(--fs-3xs)",
              letterSpacing: "var(--ls-wide)",
            }}
          >
            error
          </span>
        )}
      </header>
      {input.description && (
        <div
          style={{
            padding: "var(--sp-4) var(--sp-10)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-muted)",
            fontStyle: "italic",
          }}
        >
          {input.description}
        </div>
      )}
      {stdout.length > 0 && (
        <Block label="stdout" text={stdout} tone="default" />
      )}
      {stderr.length > 0 && (
        <Block label="stderr" text={stderr} tone="warn" />
      )}
      {stdout.length === 0 && stderr.length === 0 && (
        <div
          style={{
            padding: "var(--sp-10)",
            color: "var(--fg-ghost)",
            fontSize: "var(--fs-xs)",
            fontStyle: "italic",
          }}
        >
          (no output yet)
        </div>
      )}
    </div>
  );
}

function Block({
  label,
  text,
  tone,
}: {
  label: string;
  text: string;
  tone: "default" | "warn";
}) {
  const shown = text.slice(0, OUTPUT_CLAMP);
  const hidden = Math.max(0, text.length - shown.length);
  return (
    <section>
      <div
        style={{
          padding: "var(--sp-4) var(--sp-10)",
          fontSize: "var(--fs-3xs)",
          color: tone === "warn" ? "var(--warn)" : "var(--fg-faint)",
          letterSpacing: "var(--ls-wide)",
          textTransform: "uppercase",
          background: "var(--bg)",
          borderTop: "var(--bw-hair) solid var(--line)",
        }}
      >
        {label}
      </div>
      <pre
        className="mono"
        style={{
          margin: 0,
          padding: "var(--sp-6) var(--sp-10)",
          fontSize: "var(--fs-xs)",
          color: tone === "warn" ? "var(--warn)" : "var(--fg)",
          whiteSpace: "pre-wrap",
          wordBreak: "break-word",
          maxHeight: "300px",
          overflow: "auto",
        }}
      >
        {shown}
        {hidden > 0 && `\n… ${hidden} chars hidden`}
      </pre>
    </section>
  );
}
