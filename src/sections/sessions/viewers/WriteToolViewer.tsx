import type { LinkedTool } from "../../../types";
import { Glyph } from "../../../components/primitives/Glyph";
import { NF } from "../../../icons";
import { redactSecrets } from "./redact";
import { parseToolInput, type WriteInput } from "./toolInput";

const CONTENT_CLAMP = 4000;

/**
 * Write tool viewer — render the target path and the new content body.
 * Truncates long content but gives the user a disclosure to see the
 * rest via the transcript modal.
 */
export function WriteToolViewer({ tool }: { tool: LinkedTool }) {
  const parsed = parseToolInput<WriteInput>(tool.input_preview);
  const input = parsed.ok ? parsed.value : {};
  const path = input.file_path ?? "(unknown file)";
  const content = redactSecrets(input.content ?? "");
  const shown = content.slice(0, CONTENT_CLAMP);
  const clamped = content.length > CONTENT_CLAMP;

  return (
    <div
      data-testid="write-tool-viewer"
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
        <Glyph g={NF.fileCode} style={{ fontSize: "var(--fs-sm)" }} />
        <span
          className="mono"
          style={{
            flex: 1,
            minWidth: 0,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
          title={path}
        >
          {path}
        </span>
        <span style={{ color: "var(--fg-ghost)" }}>
          {content.length.toLocaleString()} chars
        </span>
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
      <pre
        className="mono"
        style={{
          margin: 0,
          padding: "var(--sp-6) var(--sp-10)",
          fontSize: "var(--fs-xs)",
          color: "var(--fg)",
          whiteSpace: "pre-wrap",
          wordBreak: "break-word",
          maxHeight: "var(--viewer-max-height)",
          overflow: "auto",
        }}
      >
        {shown}
        {clamped && `\n… ${content.length - shown.length} chars hidden`}
      </pre>
    </div>
  );
}
