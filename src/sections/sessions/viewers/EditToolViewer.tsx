import type { LinkedTool } from "../../../types";
import { Glyph } from "../../../components/primitives/Glyph";
import { NF } from "../../../icons";
import { computeDiff, parseToolInput, type EditInput } from "./toolInput";

/**
 * Edit tool viewer — renders the paper-mono inline diff for an
 * `Edit`/`MultiEdit`-style call. The input preview is clipped to 240
 * chars by the Rust parser; when it is incomplete we fall back to a
 * plain JSON dump. The result pane reuses the tool result body
 * verbatim (usually CC echoes the patch — we trust it).
 */
export function EditToolViewer({ tool }: { tool: LinkedTool }) {
  const parsed = parseToolInput<EditInput>(tool.input_preview);
  if (!parsed.ok) {
    return <RawFallback tool={tool} rawInput={parsed.raw} />;
  }
  const { file_path, old_string, new_string, replace_all } = parsed.value;
  if (!file_path || old_string == null || new_string == null) {
    return <RawFallback tool={tool} rawInput={tool.input_preview} />;
  }
  const diff = computeDiff(old_string, new_string);

  return (
    <div
      data-testid="edit-tool-viewer"
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
        <Glyph g={NF.edit} style={{ fontSize: "var(--fs-sm)" }} />
        <span
          className="mono"
          title={file_path}
          style={{
            flex: 1,
            minWidth: 0,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {file_path}
        </span>
        {replace_all && (
          <span
            style={{
              textTransform: "uppercase",
              letterSpacing: "var(--ls-wide)",
              color: "var(--accent-ink)",
              fontSize: "var(--fs-3xs)",
            }}
          >
            replace all
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
      <div
        className="mono"
        style={{
          padding: "var(--sp-6) var(--sp-10)",
          fontSize: "var(--fs-xs)",
          lineHeight: "var(--lh-body)",
        }}
      >
        {diff.map((line, i) => (
          <div
            key={i}
            style={{
              display: "grid",
              gridTemplateColumns: "3ch 3ch 1fr",
              gap: "var(--sp-6)",
              padding: "0 var(--sp-4)",
              background: backgroundFor(line.kind),
              color: colorFor(line.kind),
            }}
          >
            <span style={{ color: "var(--fg-ghost)" }}>
              {line.oldLine ?? ""}
            </span>
            <span style={{ color: "var(--fg-ghost)" }}>
              {line.newLine ?? ""}
            </span>
            <span
              style={{ whiteSpace: "pre", overflowX: "auto" }}
            >{`${marker(line.kind)} ${line.text}`}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function backgroundFor(kind: "add" | "remove" | "context"): string {
  if (kind === "add") return "color-mix(in oklch, var(--ok) 15%, transparent)";
  if (kind === "remove") return "color-mix(in oklch, var(--warn) 15%, transparent)";
  return "transparent";
}

function colorFor(kind: "add" | "remove" | "context"): string {
  if (kind === "add") return "var(--ok)";
  if (kind === "remove") return "var(--warn)";
  return "var(--fg)";
}

function marker(kind: "add" | "remove" | "context"): string {
  if (kind === "add") return "+";
  if (kind === "remove") return "-";
  return " ";
}

function RawFallback({ tool, rawInput }: { tool: LinkedTool; rawInput: string }) {
  return (
    <div
      data-testid="edit-tool-viewer-fallback"
      className="mono"
      style={{
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        padding: "var(--sp-8)",
        background: "var(--bg-raised)",
        fontSize: "var(--fs-xs)",
        whiteSpace: "pre-wrap",
      }}
    >
      <strong>Edit</strong> {tool.tool_use_id.slice(0, 8)}
      {"\n"}
      {rawInput}
    </div>
  );
}
