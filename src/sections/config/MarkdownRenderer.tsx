import { type ReactNode, useMemo } from "react";

/**
 * Minimal, dependency-free Markdown renderer for Config previews.
 *
 * Supports the subset we actually need for CC artifacts (CLAUDE.md,
 * agents, rules, commands, memory files): YAML frontmatter (rendered
 * as a separate card), H1–H3 headings, bulleted lists, numbered lists,
 * inline `code`, fenced code blocks, bold/italic, and paragraphs.
 * Anything richer falls back to plain paragraphs.
 *
 * SECURITY: the input has already passed through
 * `claudepot_core::config_view::mask::mask_bytes`, so no secret leaks.
 * We additionally text-escape every user string before insertion —
 * the renderer never constructs HTML from input.
 */
export function MarkdownRenderer({ body }: { body: string }) {
  const { frontmatter, blocks } = useMemo(() => parseMarkdown(body), [body]);

  return (
    <article
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-12)",
        padding: "var(--sp-16) var(--sp-20)",
        fontSize: "var(--fs-sm)",
        lineHeight: 1.55,
        color: "var(--fg)",
      }}
    >
      {frontmatter && <FrontmatterCard entries={frontmatter} />}
      {blocks.map((block, i) => renderBlock(block, i))}
    </article>
  );
}

// ---------- Parser ----------------------------------------------------

type Block =
  | { kind: "h"; level: 1 | 2 | 3; text: string }
  | { kind: "p"; text: string }
  | { kind: "ul"; items: string[] }
  | { kind: "ol"; items: string[] }
  | { kind: "pre"; lang: string | null; code: string }
  | { kind: "hr" }
  | { kind: "blockquote"; text: string };

interface Parsed {
  frontmatter: { key: string; value: string }[] | null;
  blocks: Block[];
}

function parseMarkdown(input: string): Parsed {
  const { frontmatter, rest } = extractFrontmatter(input);
  const blocks: Block[] = [];
  const lines = rest.split(/\r?\n/);
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    // Fenced code block
    const fenceMatch = line.match(/^```(.*)$/);
    if (fenceMatch) {
      const lang = fenceMatch[1].trim() || null;
      const codeLines: string[] = [];
      i += 1;
      while (i < lines.length && !/^```/.test(lines[i])) {
        codeLines.push(lines[i]);
        i += 1;
      }
      i += 1; // skip closing fence
      blocks.push({ kind: "pre", lang, code: codeLines.join("\n") });
      continue;
    }

    // Horizontal rule
    if (/^(-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
      blocks.push({ kind: "hr" });
      i += 1;
      continue;
    }

    // Heading
    const headingMatch = line.match(/^(#{1,3})\s+(.*)$/);
    if (headingMatch) {
      const level = headingMatch[1].length as 1 | 2 | 3;
      blocks.push({ kind: "h", level, text: headingMatch[2] });
      i += 1;
      continue;
    }

    // Blockquote (single-line; lumpy paragraphs fold into one block)
    if (/^>\s?/.test(line)) {
      const quoted: string[] = [];
      while (i < lines.length && /^>\s?/.test(lines[i])) {
        quoted.push(lines[i].replace(/^>\s?/, ""));
        i += 1;
      }
      blocks.push({ kind: "blockquote", text: quoted.join(" ") });
      continue;
    }

    // Unordered list
    if (/^[-*+]\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^[-*+]\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^[-*+]\s+/, ""));
        i += 1;
      }
      blocks.push({ kind: "ul", items });
      continue;
    }

    // Ordered list
    if (/^\d+\.\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^\d+\.\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^\d+\.\s+/, ""));
        i += 1;
      }
      blocks.push({ kind: "ol", items });
      continue;
    }

    // Blank line — paragraph separator
    if (line.trim() === "") {
      i += 1;
      continue;
    }

    // Paragraph — collect until blank line / structural delimiter
    const para: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !/^(#{1,3}\s|```|>\s?|[-*+]\s|\d+\.\s|-{3,}\s*$|\*{3,}\s*$|_{3,}\s*$)/.test(
        lines[i],
      )
    ) {
      para.push(lines[i]);
      i += 1;
    }
    if (para.length > 0) {
      blocks.push({ kind: "p", text: para.join(" ") });
    }
  }

  return { frontmatter, blocks };
}

function extractFrontmatter(input: string): {
  frontmatter: { key: string; value: string }[] | null;
  rest: string;
} {
  const m = input.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?/);
  if (!m) return { frontmatter: null, rest: input };
  const entries: { key: string; value: string }[] = [];
  for (const line of m[1].split(/\r?\n/)) {
    const kv = line.match(/^([A-Za-z][A-Za-z0-9_-]*):\s?(.*)$/);
    if (!kv) continue;
    entries.push({
      key: kv[1],
      value: kv[2].trim().replace(/^"(.*)"$/, "$1").replace(/^'(.*)'$/, "$1"),
    });
  }
  return { frontmatter: entries, rest: input.slice(m[0].length) };
}

// ---------- Renderers -------------------------------------------------

function FrontmatterCard({
  entries,
}: {
  entries: { key: string; value: string }[];
}) {
  if (entries.length === 0) return null;
  return (
    <aside
      aria-label="Frontmatter"
      style={{
        display: "grid",
        gridTemplateColumns: "auto 1fr",
        columnGap: "var(--sp-12)",
        rowGap: "var(--sp-3)",
        padding: "var(--sp-10) var(--sp-12)",
        background: "var(--bg-sunken)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        fontFamily: "var(--mono)",
        fontSize: "var(--fs-xs)",
      }}
    >
      {entries.map((e) => (
        <Row key={e.key} k={e.key} v={e.value} />
      ))}
    </aside>
  );
}

function Row({ k, v }: { k: string; v: string }) {
  return (
    <>
      <span style={{ color: "var(--fg-faint)" }}>{k}</span>
      <span style={{ color: "var(--fg)" }}>{v || <em>(empty)</em>}</span>
    </>
  );
}

function renderBlock(block: Block, key: number): ReactNode {
  switch (block.kind) {
    case "h": {
      const size =
        block.level === 1
          ? "var(--fs-md-lg)"
          : block.level === 2
            ? "var(--fs-md)"
            : "var(--fs-sm)";
      const Tag: "h3" | "h4" | "h5" = `h${block.level + 2}` as
        | "h3"
        | "h4"
        | "h5";
      return (
        <Tag
          key={key}
          style={{
            margin: "var(--sp-6) 0 0",
            fontSize: size,
            fontWeight: 600,
            color: "var(--fg)",
          }}
        >
          {renderInline(block.text)}
        </Tag>
      );
    }
    case "p":
      return (
        <p key={key} style={{ margin: 0 }}>
          {renderInline(block.text)}
        </p>
      );
    case "ul":
      return (
        <ul
          key={key}
          style={{ margin: 0, paddingLeft: "var(--sp-20)", display: "flex", flexDirection: "column", gap: "var(--sp-3)" }}
        >
          {block.items.map((it, i) => (
            <li key={i}>{renderInline(it)}</li>
          ))}
        </ul>
      );
    case "ol":
      return (
        <ol
          key={key}
          style={{ margin: 0, paddingLeft: "var(--sp-20)", display: "flex", flexDirection: "column", gap: "var(--sp-3)" }}
        >
          {block.items.map((it, i) => (
            <li key={i}>{renderInline(it)}</li>
          ))}
        </ol>
      );
    case "pre":
      return (
        <pre
          key={key}
          style={{
            margin: 0,
            padding: "var(--sp-10) var(--sp-12)",
            background: "var(--bg-sunken)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-2)",
            fontFamily: "var(--mono)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg)",
            overflow: "auto",
            whiteSpace: "pre",
          }}
          aria-label={block.lang ? `${block.lang} code` : "code"}
        >
          {block.code}
        </pre>
      );
    case "hr":
      return (
        <hr
          key={key}
          style={{
            border: "none",
            borderTop: "var(--bw-hair) solid var(--line)",
            margin: "var(--sp-6) 0",
          }}
        />
      );
    case "blockquote":
      return (
        <blockquote
          key={key}
          style={{
            margin: 0,
            padding: "var(--sp-4) var(--sp-12)",
            borderLeft: "var(--bw-strong) solid var(--accent-border)",
            color: "var(--fg-muted)",
            fontStyle: "italic",
          }}
        >
          {renderInline(block.text)}
        </blockquote>
      );
  }
}

// ---------- Inline span rendering ------------------------------------

/**
 * Render the inline subset: `code`, **bold**, *italic*, [text](url).
 * Plain-text insertion only — no HTML strings are ever constructed
 * from input.
 */
function renderInline(text: string): ReactNode {
  const out: ReactNode[] = [];
  const re =
    /`([^`]+)`|\*\*([^*]+)\*\*|__([^_]+)__|\*([^*]+)\*|_([^_]+)_|\[([^\]]+)\]\(([^)]+)\)/g;
  let last = 0;
  let match: RegExpExecArray | null;
  let key = 0;
  while ((match = re.exec(text)) !== null) {
    if (match.index > last) out.push(text.slice(last, match.index));
    if (match[1] !== undefined) {
      out.push(
        <code
          key={`c${key++}`}
          style={{
            fontFamily: "var(--mono)",
            fontSize: "0.92em",
            padding: "0 var(--sp-3)",
            background: "var(--bg-sunken)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-1)",
          }}
        >
          {match[1]}
        </code>,
      );
    } else if (match[2] !== undefined || match[3] !== undefined) {
      out.push(
        <strong key={`b${key++}`}>
          {match[2] ?? match[3]}
        </strong>,
      );
    } else if (match[4] !== undefined || match[5] !== undefined) {
      out.push(<em key={`i${key++}`}>{match[4] ?? match[5]}</em>);
    } else if (match[6] !== undefined && match[7] !== undefined) {
      // Link — render as text + href in a title. We never generate
      // anchors with raw URLs (Tauri webview has no navigation target).
      out.push(
        <span
          key={`l${key++}`}
          title={match[7]}
          style={{ color: "var(--accent-ink)", textDecoration: "underline" }}
        >
          {match[6]}
        </span>,
      );
    }
    last = re.lastIndex;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}
