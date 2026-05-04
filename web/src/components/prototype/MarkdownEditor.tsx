"use client";

import { useId, useRef, useState, type ChangeEvent, type ComponentType } from "react";
import { Bold, Code, Eye, Italic, Link as LinkIcon, List, Pencil, Quote } from "lucide-react";
import { renderMarkdown } from "@/lib/markdown";

/**
 * GitHub-style Markdown editor: Write/Preview tabs + a thin formatting
 * toolbar. The textarea is always the source of truth; Preview just
 * renders the current value through the same `renderMarkdown` helper
 * the server uses to render comments and submission bodies, so what
 * the user sees here is what will appear in the feed.
 *
 * The toolbar offers: Bold, Italic, Code, Link, List, Quote.  Headings
 * and images are intentionally absent — `renderMarkdown` strips them
 * out via its tag allowlist (see src/lib/markdown.ts), so a button
 * that produces `# heading` would just render as plain text.
 *
 * Form integration:
 *   - Pass `name` for native form-data submit (used in /submit).
 *   - Pass `value` + `onChange` for controlled state (used in
 *     CommentForm). Both modes can coexist.
 *
 * The textarea is kept in the DOM in both modes (offscreen when
 * Preview is active) so HTML5 `required` validation continues to
 * work — browsers refuse to focus a `display: none` field, which
 * would silently break the submit flow.
 */

type Mode = "write" | "preview";
type Op = "bold" | "italic" | "code" | "link" | "list" | "quote";

interface Props {
  // Form integration — mutually compatible
  name?: string;
  value?: string;
  onChange?: (next: string) => void;
  defaultValue?: string;
  // Standard textarea props
  rows?: number;
  maxLength?: number;
  placeholder?: string;
  required?: boolean;
  disabled?: boolean;
  // Wrapping div className passthrough (host can size/space the editor)
  className?: string;
}

/**
 * React tracks textarea value via its internal property descriptor.
 * Setting `el.value = next` directly bypasses React's change detection,
 * so toolbar inserts in a controlled component would not propagate.
 * The native setter dispatched alongside an `input` event is the
 * canonical workaround.
 */
function setTextareaValue(el: HTMLTextAreaElement, next: string): void {
  if (typeof window === "undefined") return;
  const setter = Object.getOwnPropertyDescriptor(
    window.HTMLTextAreaElement.prototype,
    "value",
  )?.set;
  if (setter) {
    setter.call(el, next);
  } else {
    el.value = next;
  }
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

function applyOp(el: HTMLTextAreaElement, op: Op): void {
  const { selectionStart: s, selectionEnd: e, value: v } = el;
  const selected = v.slice(s, e);
  let next = v;
  let nextStart = s;
  let nextEnd = e;

  switch (op) {
    case "bold": {
      const insert = selected || "bold text";
      next = `${v.slice(0, s)}**${insert}**${v.slice(e)}`;
      nextStart = s + 2;
      nextEnd = nextStart + insert.length;
      break;
    }
    case "italic": {
      const insert = selected || "italic text";
      next = `${v.slice(0, s)}*${insert}*${v.slice(e)}`;
      nextStart = s + 1;
      nextEnd = nextStart + insert.length;
      break;
    }
    case "code": {
      // Multi-line selection → fenced block; otherwise inline ticks.
      if (selected.includes("\n")) {
        const block = `\n\`\`\`\n${selected}\n\`\`\`\n`;
        next = `${v.slice(0, s)}${block}${v.slice(e)}`;
        nextStart = s + block.length;
        nextEnd = nextStart;
      } else {
        const insert = selected || "code";
        next = `${v.slice(0, s)}\`${insert}\`${v.slice(e)}`;
        nextStart = s + 1;
        nextEnd = nextStart + insert.length;
      }
      break;
    }
    case "link": {
      const text = selected || "link text";
      next = `${v.slice(0, s)}[${text}](https://)${v.slice(e)}`;
      // Cursor on the URL placeholder so the user can paste over it.
      const urlStart = s + text.length + 3; // [text](
      nextStart = urlStart;
      nextEnd = urlStart + 8; // length of "https://"
      break;
    }
    case "list":
    case "quote": {
      const prefix = op === "list" ? "- " : "> ";
      // Find the boundaries of the selected lines.
      const lineStart = v.lastIndexOf("\n", s - 1) + 1;
      const lineEnd = v.indexOf("\n", e);
      const blockEnd = lineEnd === -1 ? v.length : lineEnd;
      const block = v.slice(lineStart, blockEnd);
      const prefixed = block
        .split("\n")
        .map((line) => `${prefix}${line}`)
        .join("\n");
      next = `${v.slice(0, lineStart)}${prefixed}${v.slice(blockEnd)}`;
      nextStart = lineStart;
      nextEnd = lineStart + prefixed.length;
      break;
    }
  }

  setTextareaValue(el, next);
  // Selection restoration must run after React re-renders the value.
  requestAnimationFrame(() => {
    el.focus();
    el.setSelectionRange(nextStart, nextEnd);
  });
}

type IconCmp = ComponentType<{ size?: number; "aria-hidden"?: boolean }>;
const TOOLBAR: Array<{ op: Op; label: string; title: string; Icon: IconCmp }> = [
  { op: "bold", label: "Bold", title: "Bold (**text**)", Icon: Bold },
  { op: "italic", label: "Italic", title: "Italic (*text*)", Icon: Italic },
  { op: "code", label: "Code", title: "Code (`text`)", Icon: Code },
  { op: "link", label: "Link", title: "Link [text](url)", Icon: LinkIcon },
  { op: "list", label: "List", title: "List (- item)", Icon: List },
  { op: "quote", label: "Quote", title: "Quote (> text)", Icon: Quote },
];

export function MarkdownEditor({
  name,
  value: controlledValue,
  onChange,
  defaultValue = "",
  rows = 6,
  maxLength,
  placeholder,
  required,
  disabled,
  className,
}: Props) {
  const isControlled = controlledValue !== undefined;
  const [internal, setInternal] = useState(defaultValue);
  const value = isControlled ? controlledValue : internal;
  const [mode, setMode] = useState<Mode>("write");
  const ref = useRef<HTMLTextAreaElement>(null);
  const id = useId();

  function handleChange(e: ChangeEvent<HTMLTextAreaElement>) {
    if (!isControlled) setInternal(e.target.value);
    onChange?.(e.target.value);
  }

  function fire(op: Op) {
    const el = ref.current;
    if (!el || disabled) return;
    if (mode !== "write") setMode("write");
    applyOp(el, op);
  }

  return (
    <div className={`proto-md-editor ${className ?? ""}`.trim()}>
      <div className="proto-md-bar">
        <div className="proto-md-tabs" role="tablist" aria-label="Editor mode">
          <button
            type="button"
            role="tab"
            aria-selected={mode === "write"}
            aria-controls={`${id}-area`}
            aria-label="Write"
            title="Write"
            className={`proto-md-tab ${mode === "write" ? "proto-md-tab--active" : ""}`.trim()}
            onClick={() => setMode("write")}
          >
            <Pencil size={14} aria-hidden />
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={mode === "preview"}
            aria-controls={`${id}-area`}
            aria-label="Preview"
            title="Preview"
            className={`proto-md-tab ${mode === "preview" ? "proto-md-tab--active" : ""}`.trim()}
            onClick={() => setMode("preview")}
          >
            <Eye size={14} aria-hidden />
          </button>
        </div>
        <div className="proto-md-toolbar" role="toolbar" aria-label="Formatting">
          {TOOLBAR.map((b) => (
            <button
              key={b.op}
              type="button"
              className="proto-md-tool"
              aria-label={b.label}
              title={b.title}
              onClick={() => fire(b.op)}
              disabled={disabled || mode !== "write"}
            >
              <b.Icon size={14} aria-hidden />
            </button>
          ))}
        </div>
      </div>

      <div className="proto-md-area" id={`${id}-area`}>
        <textarea
          ref={ref}
          name={name}
          value={value}
          onChange={handleChange}
          rows={rows}
          maxLength={maxLength}
          placeholder={placeholder}
          required={required}
          disabled={disabled}
          className={`proto-md-textarea ${mode === "preview" ? "proto-md-textarea--offscreen" : ""}`.trim()}
        />
        {mode === "preview" ? (
          <div
            className="proto-md-preview"
            // The same `renderMarkdown` runs server-side at submit
            // time. Skipping sanitization here would diverge the two,
            // so we eat the ~50 KB bundle cost and render identically.
            dangerouslySetInnerHTML={{
              __html: value.trim()
                ? renderMarkdown(value)
                : `<p class="proto-md-preview-empty">Nothing to preview yet.</p>`,
            }}
          />
        ) : null}
      </div>
    </div>
  );
}
