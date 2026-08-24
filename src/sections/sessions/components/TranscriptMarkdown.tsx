import { Children, isValidElement, type ReactElement, type ReactNode } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { ExternalLink } from "../../../components/primitives/ExternalLink";
import { MermaidBlock } from "../../config/MermaidBlock";
import { extractCodeText } from "../../config/extractCodeText";
import { hasLanguage } from "../../../lib/codeFence";
import { isSafeHref } from "../../../lib/mdLink";
import i18n from "../../../lib/i18n";

/**
 * Markdown for a transcript turn.
 *
 * Claude writes markdown, and this viewer showed it verbatim: `**bold**`
 * with the asterisks, and a GFM table as one run-on line of pipes.
 *
 * Deliberately **not** `config/MarkdownRenderer`. That one is for
 * browsing a document — it splits frontmatter into a card and runs
 * highlight.js. A transcript turn is a message: a `---` in it is a
 * horizontal rule the model typed, not metadata. Reusing that renderer
 * would change what the transcript *says*.
 *
 * It does share that renderer's `MermaidBlock`. An earlier version of
 * this file argued a fenced diagram was "something the model wrote about
 * rather than a diagram to draw" — that was wrong about how these
 * transcripts are used: a diagram in an answer is the answer, and
 * reading it as source is reading the wrong artifact.
 *
 * It shares that renderer's stylesheet, though. `.md-body` already
 * carries every element rule and passes the token gate; `.md-transcript`
 * only drops the padding and border-radius that suit a document pane and
 * not a chat bubble.
 *
 * SECURITY — the input is model output, plus whatever it quoted from
 * disk, already through `redactSecrets` at the call site:
 *   - No `rehype-raw`, so embedded HTML renders as escaped text. Same
 *     guarantee `MarkdownRenderer` documents.
 *   - No `dangerouslySetInnerHTML` in this file.
 *   - react-markdown's default URL transform drops everything outside
 *     http / https / mailto / tel, so a `javascript:` link is inert text.
 *   - Links go through `ExternalLink`, which hands the URL to the OS
 *     opener. A bare `<a href>` inside a Tauri webview navigates **the
 *     application itself** to the URL — the window stops being Claudepot
 *     and there is no back button. That is the one difference from the
 *     remote panel, where a new tab is the correct answer.
 *   - Images render as their alt text. The webview should not fetch a
 *     remote image because a transcript names one, and a local `file://`
 *     reference is not ours to open either.
 *   - `MermaidBlock` runs mermaid with `securityLevel: "strict"`, which
 *     disables embedded HTML, click handlers and script in diagram
 *     source, and lazy-imports the library so a transcript with no
 *     diagram never loads it.
 */
export function TranscriptMarkdown({ body }: { body: string }) {
  return (
    <div className="md-body md-transcript">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {body}
      </ReactMarkdown>
    </div>
  );
}

const components: Components = {
  // `pre`, not `code`: a diagram replaces the whole block, and the
  // default `<pre>` shell would box the SVG inside scrollbars and a
  // code-block border.
  pre(props) {
    // react-markdown passes a non-DOM `node` prop alongside DOM attrs;
    // spreading it onto `<pre>` emits `node="[object Object]"`.
    const { children, node: _node, ...rest } = props as typeof props & {
      node?: unknown;
    };
    const codeChild = Children.toArray(children).find(isValidElement) as
      | ReactElement<{ className?: string; children?: ReactNode }>
      | undefined;

    if (hasLanguage(codeChild?.props?.className, "mermaid")) {
      return <MermaidBlock source={extractCodeText(codeChild?.props?.children)} />;
    }
    return <pre {...rest}>{children}</pre>;
  },

  // Only http(s) and mailto reach the OS opener. See `isSafeHref` —
  // react-markdown's default lets `irc:`/`xmpp:`/relative through, and
  // this renderer hands the value to `openUrl`.
  a: ({ href, children }) =>
    isSafeHref(href) ? (
      <ExternalLink href={href as string}>{children}</ExternalLink>
    ) : (
      <span title={href ?? undefined}>{children}</span>
    ),
  img: ({ alt, src }) => (
    <span
      className="md-img-alt"
      title={typeof src === "string" ? src : undefined}
    >
      {alt || i18n.t("transcript.imageAlt", { ns: "sessions" })}
    </span>
  ),
  // A wide table must scroll inside its own wrapper. Without this it
  // widens the bubble and every sibling row reflows around it.
  table: ({ children }) => (
    <div className="md-table-scroll">
      <table>{children}</table>
    </div>
  ),
};
