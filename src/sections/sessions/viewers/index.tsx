import { useState } from "react";
import { useTranslation } from "react-i18next";
import { i18n } from "../../../lib/i18n";
import type { LinkedTool } from "../../../types";
import { Glyph } from "../../../components/primitives/Glyph";
import { NF } from "../../../icons";
import { CopyButton } from "../../../components/CopyButton";
import { BashToolViewer } from "./BashToolViewer";
import { EditToolViewer } from "./EditToolViewer";
import { ReadToolViewer } from "./ReadToolViewer";
import { WriteToolViewer } from "./WriteToolViewer";
import { redactSecrets } from "../../../lib/redactSecrets";

const GENERIC_CLAMP = 4000;

/**
 * Dispatcher — chooses the specialized tool viewer by tool name and
 * falls back to a compact default card for anything we don't have a
 * bespoke renderer for. Keeping the dispatch here means consumers
 * only need one import.
 */
export function ToolExecutionView({ tool }: { tool: LinkedTool }) {
  switch (tool.tool_name) {
    case "Edit":
    case "MultiEdit":
      return <EditToolViewer tool={tool} />;
    case "Read":
      return <ReadToolViewer tool={tool} />;
    case "Write":
      return <WriteToolViewer tool={tool} />;
    case "Bash":
      return <BashToolViewer tool={tool} />;
    default:
      return <GenericToolView tool={tool} />;
  }
}

function GenericToolView({ tool }: { tool: LinkedTool }) {
  const { t } = useTranslation("sessions");
  // Clipboard payload: input on top, result below, separated by a
  // visual rule. Mirrors what's rendered in the card so the user gets
  // the same content they can read.
  const copyPieces: string[] = [];
  const input = redactSecrets(tool.input_preview);
  if (input) copyPieces.push(`[input]\n${input}`);
  if (tool.result_content != null) {
    copyPieces.push(`[result]\n${redactSecrets(tool.result_content)}`);
  }
  const copyText = copyPieces.join("\n\n");
  return (
    <div
      data-testid={`tool-viewer-${tool.tool_name.toLowerCase()}`}
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
        <Glyph g={NF.wrench} style={{ fontSize: "var(--fs-sm)" }} />
        <span style={{ flex: 1 }}>{tool.tool_name}</span>
        {tool.duration_ms != null && (
          <span style={{ color: "var(--fg-faint)" }}>
            {formatDuration(tool.duration_ms)}
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
            {t("viewer.error")}
          </span>
        )}
        {tool.result_content == null && (
          <span
            style={{
              color: "var(--fg-faint)",
              fontSize: "var(--fs-3xs)",
              letterSpacing: "var(--ls-wide)",
              textTransform: "uppercase",
            }}
          >
            {t("viewer.generic.orphan")}
          </span>
        )}
        {copyText.length > 0 && (
          <CopyButton text={copyText} ariaLabel={t("viewer.generic.copyAria")} />
        )}
      </header>
      <div
        className="mono"
        style={{
          padding: "var(--sp-6) var(--sp-10)",
          fontSize: "var(--fs-xs)",
          color: "var(--fg-muted)",
        }}
      >
        <div style={{ color: "var(--fg-faint)" }}>{t("viewer.generic.input")}</div>
        <ClampedPre text={redactSecrets(tool.input_preview)} />
        {tool.result_content != null && (
          <>
            <div style={{ color: "var(--fg-faint)", marginTop: "var(--sp-6)" }}>
              {t("viewer.generic.result")}
            </div>
            <ClampedPre text={redactSecrets(tool.result_content)} />
          </>
        )}
      </div>
    </div>
  );
}

/**
 * `<pre>` with a soft cap on rendered length. An unknown tool can dump
 * megabytes into `result_content`; rendering all of it in the transcript
 * pane freezes the webview. We cut at GENERIC_CLAMP chars by default
 * and let the user opt-in to the full body.
 */
function ClampedPre({ text }: { text: string }) {
  const { t } = useTranslation("sessions");
  const [expanded, setExpanded] = useState(false);
  const clamp = text.length > GENERIC_CLAMP && !expanded;
  const shown = clamp ? text.slice(0, GENERIC_CLAMP) : text;
  const hidden = text.length - shown.length;
  return (
    <div>
      <pre
        style={{
          margin: 0,
          whiteSpace: "pre-wrap",
          wordBreak: "break-word",
          maxHeight: "var(--viewer-max-height)",
          overflow: "auto",
        }}
      >
        {shown}
        {clamp && `\n${t("viewer.charsHidden", { n: hidden })}`}
      </pre>
      {text.length > GENERIC_CLAMP && (
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
            letterSpacing: "var(--ls-wide)",
            textTransform: "uppercase",
          }}
        >
          {expanded
            ? t("viewer.collapse")
            : t("viewer.showMoreChars", { n: hidden })}
        </button>
      )}
    </div>
  );
}

// Render-time lookup via the shared i18n instance (this is a plain
// helper, not a component) so language switches take effect on the
// next render.
function formatDuration(ms: number): string {
  if (ms < 1000) {
    return i18n.t("viewer.generic.durationMs", { ns: "sessions", n: ms });
  }
  const s = ms / 1000;
  if (s < 60) {
    return i18n.t("viewer.generic.durationSec", {
      ns: "sessions",
      n: s.toFixed(1),
    });
  }
  const m = s / 60;
  return i18n.t("viewer.generic.durationMin", {
    ns: "sessions",
    n: m.toFixed(1),
  });
}

export { EditToolViewer, ReadToolViewer, WriteToolViewer, BashToolViewer };
