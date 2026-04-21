import type { LinkedTool } from "../../../types";
import { Glyph } from "../../../components/primitives/Glyph";
import { NF } from "../../../icons";
import { parseToolInput, type ReadInput } from "./toolInput";

const MAX_LINES = 200;

/**
 * Read tool viewer — renders the requested file path + range header
 * and a syntax-off (paper-mono text) body with line numbers. CC's
 * Read tool inlines the file content as `<line> \t <content>` — we
 * split by newline and preserve the original numbering.
 */
export function ReadToolViewer({ tool }: { tool: LinkedTool }) {
  const parsed = parseToolInput<ReadInput>(tool.input_preview);
  const input = parsed.ok ? parsed.value : {};
  const path = input.file_path ?? "(unknown file)";

  const body = tool.result_content ?? "";
  const lines = body.split("\n");
  const shown = lines.slice(0, MAX_LINES);
  const hidden = Math.max(0, lines.length - shown.length);

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
          {shown.map((line, i) => (
            <div
              key={i}
              style={{
                display: "grid",
                gridTemplateColumns: "4ch 1fr",
                gap: "var(--sp-6)",
              }}
            >
              <span style={{ color: "var(--fg-ghost)" }}>{i + 1}</span>
              <span style={{ whiteSpace: "pre", overflowX: "auto" }}>
                {line}
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
