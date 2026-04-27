import {
  Children,
  isValidElement,
  useMemo,
  type ReactElement,
  type ReactNode,
} from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { MermaidBlock } from "./MermaidBlock";

/**
 * Markdown renderer for Config previews.
 *
 * Pipeline:
 *   raw → frontmatter split → react-markdown → remark-gfm
 *                                            → rehype-highlight (lowlight)
 *                                            → `pre` override:
 *                                                language-mermaid → MermaidBlock
 *                                                else             → highlighted <pre>
 *
 * The frontmatter card is paper-mono UI, not a markdown construct, so
 * we strip a leading `---\n…\n---\n` block and render it as a separate
 * key/value card before handing the rest to react-markdown.
 *
 * SECURITY:
 *   - Input has already been masked by `claudepot_core::config_view::mask`,
 *     so no secret leaks.
 *   - react-markdown does NOT use `dangerouslySetInnerHTML` for input
 *     text, and we do not enable `rehype-raw`, so embedded HTML in the
 *     source is rendered as escaped text. Same guarantee as the
 *     previous hand-rolled renderer.
 *   - rehype-highlight runs on the parsed mdast/hast tree (not on
 *     strings), so it cannot inject markup either.
 *   - MermaidBlock runs mermaid with `securityLevel: "strict"`, which
 *     disables embedded HTML, click handlers, and arbitrary script in
 *     diagram source.
 */
export function MarkdownRenderer({ body }: { body: string }) {
  const { frontmatter, rest } = useMemo(() => extractFrontmatter(body), [body]);

  return (
    <article className="md-body">
      {frontmatter && <FrontmatterCard entries={frontmatter} />}
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[[rehypeHighlight, { detect: true, ignoreMissing: true }]]}
        components={components}
      >
        {rest}
      </ReactMarkdown>
    </article>
  );
}

// ---------- Component overrides --------------------------------------

/**
 * react-markdown wraps fenced code as `<pre><code class="language-X">…</code></pre>`.
 * We override `pre` (rather than `code`) so that for mermaid we can
 * replace the entire `<pre>` shell with a diagram container — the
 * default `<pre>` styling would otherwise box the SVG inside scroll
 * bars and a code-block border.
 *
 * `a` and `img` are inert by design:
 *   - The Tauri webview will navigate away from the SPA on a real
 *     `<a href>` click. We render the link text with the URL in a
 *     tooltip so users can copy the destination without leaving the
 *     app. Opening external URLs is the editor's job, not the
 *     preview pane's.
 *   - Markdown `![alt](url)` would otherwise trigger a network fetch
 *     to whatever host the file references — a privacy leak in a
 *     local-config preview. We render the alt text + a domain hint
 *     and never load the image.
 */
const components: Components = {
  pre(props) {
    // react-markdown passes a non-DOM `node` prop alongside DOM
    // attrs; spreading it onto `<pre>` would emit
    // `node="[object Object]"`. Strip it explicitly.
    const { children, node: _node, ...rest } = props as typeof props & {
      node?: unknown;
    };
    const codeChild = Children.toArray(children).find(isValidElement) as
      | ReactElement<{ className?: string; children?: ReactNode }>
      | undefined;

    const className = codeChild?.props?.className ?? "";
    if (className.includes("language-mermaid")) {
      return <MermaidBlock source={extractCodeText(codeChild?.props?.children)} />;
    }

    return <pre {...rest}>{children}</pre>;
  },

  a(props) {
    const { children, href } = props;
    return (
      <span
        className="md-link"
        title={href ?? undefined}
        style={{
          color: "var(--accent-ink)",
          textDecoration: "underline",
          textUnderlineOffset: "0.18em",
          cursor: "default",
        }}
      >
        {children}
      </span>
    );
  },

  img(props) {
    const { src, alt } = props;
    const host = (() => {
      try {
        return src ? new URL(src).host : null;
      } catch {
        return null;
      }
    })();
    return (
      <span
        role="img"
        aria-label={alt || "image"}
        title={src ?? undefined}
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: "var(--sp-6)",
          padding: "var(--sp-2) var(--sp-6)",
          background: "var(--bg-sunken)",
          border: "var(--bw-hair) solid var(--line)",
          borderRadius: "var(--r-1)",
          fontSize: "0.9em",
          color: "var(--fg-muted)",
        }}
      >
        <span style={{ fontFamily: "var(--font-mono)" }}>image</span>
        {alt && <span>· {alt}</span>}
        {host && <span style={{ color: "var(--fg-faint)" }}>· {host}</span>}
      </span>
    );
  },
};

/**
 * Pull a flat string out of a `<code>` element's children. With
 * `rehype-highlight` enabled the children are typically a single text
 * node (when the language is unknown — mermaid is) or an array of
 * `<span class="hljs-…">` elements (when it's known). For unknown
 * languages with `ignoreMissing: true`, mermaid lands in the first
 * shape, but we handle both for resilience.
 */
function extractCodeText(node: ReactNode): string {
  if (typeof node === "string") return node;
  if (typeof node === "number") return String(node);
  if (node === null || node === undefined || typeof node === "boolean") return "";
  if (Array.isArray(node)) return node.map(extractCodeText).join("");
  if (isValidElement(node)) {
    const props = node.props as { children?: ReactNode };
    return extractCodeText(props.children);
  }
  return "";
}

// ---------- Frontmatter ----------------------------------------------

interface FrontmatterEntry {
  key: string;
  value: string;
}

function extractFrontmatter(input: string): {
  frontmatter: FrontmatterEntry[] | null;
  rest: string;
} {
  const m = input.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?/);
  if (!m) return { frontmatter: null, rest: input };
  const entries: FrontmatterEntry[] = [];
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

function FrontmatterCard({ entries }: { entries: FrontmatterEntry[] }) {
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
