import type { LinkedTool } from "../../../types";
import { Glyph } from "../../../components/primitives/Glyph";
import { NF } from "../../../icons";
import { redactSecrets } from "./redact";
import { parseToolInput, type ReadInput } from "./toolInput";

const MAX_LINES = 200;

interface NumberedLine {
  lineNumber: number;
  text: string;
}

/**
 * Detect CC's "line-number + tab + content" format. Returns parsed
 * lines when the first 20 non-empty lines all match; otherwise `null`
 * so the caller falls back to plain-text numbering from the offset.
 */
function parseNumberedLines(body: string): NumberedLine[] | null {
  const raw = body.split("\n");
  // Empty body: nothing to parse, caller handles.
  if (raw.length === 0) return null;
  const probe = raw.filter((l) => l.length > 0).slice(0, 20);
  if (probe.length === 0) return null;
  const re = /^(\d+)\t(.*)$/;
  if (!probe.every((l) => re.test(l))) return null;
  return raw
    .map((l) => {
      if (l.length === 0) return { lineNumber: NaN, text: "" };
      const m = re.exec(l);
      if (!m) return { lineNumber: NaN, text: l };
      return { lineNumber: Number(m[1]), text: m[2] ?? "" };
    })
    .filter((x) => Number.isFinite(x.lineNumber));
}

/**
 * Read tool viewer — renders the requested file path + range header
 * and a syntax-off (paper-mono text) body with line numbers. CC's
 * Read tool inlines the file content as `<line> \t <content>` — we
 * split by newline and preserve the original numbering.
 */
export function ReadToolViewer({ tool }: { tool: LinkedTool }) {
  const parsedInput = parseToolInput<ReadInput>(tool.input_preview);
  const input = parsedInput.ok ? parsedInput.value : {};
  const path = input.file_path ?? "(unknown file)";

  const body = redactSecrets(tool.result_content ?? "");
  // CC's Read tool already prefixes each line with its 1-based line
  // number + a tab when the result is file content. If every line we
  // see matches that shape, strip the prefix and use the embedded
  // numbers as the display numbers — that's authoritative and handles
  // offset reads correctly. Otherwise fall back to counting from
  // `offset + 1` if the caller passed one, else from 1.
  const firstLineNumber =
    typeof input.offset === "number" && input.offset > 0
      ? input.offset + 1
      : 1;
  const parsed = parseNumberedLines(body);
  const displayLines = parsed
    ? parsed.slice(0, MAX_LINES)
    : body
        .split("\n")
        .slice(0, MAX_LINES)
        .map((text, i) => ({ lineNumber: firstLineNumber + i, text }));
  const totalLines = (parsed ?? body.split("\n")).length;
  const hidden = Math.max(0, totalLines - displayLines.length);

  return (
    <div
      data-testid="read-tool-viewer"
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
        <Glyph g={NF.fileText} style={{ fontSize: "var(--fs-sm)" }} />
        <span className="mono" style={{ flex: 1 }} title={path}>
          {path}
        </span>
        {typeof input.offset === "number" && (
          <span style={{ color: "var(--fg-ghost)" }}>
            from {input.offset}
            {typeof input.limit === "number" ? ` · ${input.limit} lines` : ""}
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
      {body.length === 0 ? (
        <div
          style={{
            padding: "var(--sp-10)",
            color: "var(--fg-ghost)",
            fontSize: "var(--fs-xs)",
            fontStyle: "italic",
          }}
        >
          (no result yet)
        </div>
      ) : (
        <div
          className="mono"
          style={{
            padding: "var(--sp-6) var(--sp-10)",
            fontSize: "var(--fs-xs)",
            lineHeight: 1.5,
          }}
        >
          {displayLines.map((l, i) => (
            <div
              key={i}
              style={{
                display: "grid",
                gridTemplateColumns: "5ch 1fr",
                gap: "var(--sp-6)",
              }}
            >
              <span style={{ color: "var(--fg-ghost)" }}>{l.lineNumber}</span>
              <span style={{ whiteSpace: "pre", overflowX: "auto" }}>
                {l.text}
              </span>
            </div>
          ))}
          {hidden > 0 && (
            <div
              style={{
                color: "var(--fg-faint)",
                fontStyle: "italic",
                marginTop: "var(--sp-4)",
              }}
            >
              … {hidden} more line{hidden === 1 ? "" : "s"} hidden
            </div>
          )}
        </div>
      )}
    </div>
  );
}
