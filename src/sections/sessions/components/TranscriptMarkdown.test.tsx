import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { TranscriptMarkdown } from "./TranscriptMarkdown";

// `openUrl` is mocked globally in `src/test/setup.ts` — see the note
// there on why a per-file mock of a Tauri API is the wrong shape.
const opened = vi.mocked(openUrl);

/** The rendered HTML, for assertions about elements rather than text. */
function html(body: string): string {
  const { container } = render(<TranscriptMarkdown body={body} />);
  return container.innerHTML;
}

/**
 * Every element and every event-handler attribute actually in the DOM.
 *
 * Asserted against the DOM, not `innerHTML`, because jsdom serializes a
 * text node without escaping its quotes: escaped, inert markup comes
 * back as `&lt;img src=x onerror="alert(1)"&gt;`, and a string regex
 * cannot tell that from a live attribute. A first draft of this test did
 * exactly that and failed on correct behaviour.
 */
function mounted(body: string): { tags: string[]; handlers: string[] } {
  const { container } = render(<TranscriptMarkdown body={body} />);
  const els = Array.from(container.querySelectorAll("*"));
  return {
    tags: els.map((e) => e.tagName.toLowerCase()),
    handlers: els.flatMap((e) =>
      Array.from(e.attributes)
        .map((a) => a.name)
        .filter((n) => n.startsWith("on")),
    ),
  };
}

describe("TranscriptMarkdown", () => {
  it("renders the constructs Claude actually writes", () => {
    // Taken from a real transcript on this machine, which is where the
    // defect was found: this showed the asterisks.
    render(<TranscriptMarkdown body="Released. **v0.9.50 is building.**" />);
    const bold = screen.getByText("v0.9.50 is building.");
    expect(bold.tagName).toBe("STRONG");
    expect(screen.queryByText(/\*\*/)).not.toBeInTheDocument();
  });

  it("renders a GFM table inside a scroll wrapper", () => {
    // The worst case: this arrived as one run-on line of pipes, and a
    // table wide enough to widen the bubble reflows every sibling row.
    const out = html("| Step | Result |\n|---|---|\n| PR #1320 | 17/17 pass |");
    expect(out).toContain('class="md-table-scroll"');
    expect(out).toMatch(/<th[^>]*>Step<\/th>/);
    expect(out).toMatch(/<td[^>]*>17\/17 pass<\/td>/);
    expect(out).not.toContain("|---|");
  });

  it("renders headings, lists and code without leaving their markers", () => {
    const out = html("## Why\n\n- one\n- two\n\nrun `cargo test`");
    expect(out).toMatch(/<h2>Why<\/h2>/);
    expect(out).toMatch(/<li>one<\/li>/);
    expect(out).toMatch(/<code>cargo test<\/code>/);
    expect(out).not.toContain("## Why");
  });

  it("escapes embedded HTML instead of mounting it", () => {
    // The input is model output quoting arbitrary files. Assertions are
    // about elements and attributes, not substrings: the escaped form
    // `onerror=&quot;…&quot;` is inert and must not be mistaken for a
    // failure.
    for (const hostile of [
      "<script>alert(1)</script>",
      '<img src=x onerror="alert(1)">',
      '<iframe src="https://evil.example"></iframe>',
      '<div onclick="alert(1)">click</div>',
    ]) {
      const { tags, handlers } = mounted(hostile);
      expect(tags).not.toContain("script");
      expect(tags).not.toContain("iframe");
      expect(tags).not.toContain("img");
      expect(handlers).toEqual([]);
      // …and it is visible as text, not swallowed.
      expect(html(hostile)).toContain("&lt;");
    }
  });

  it("does not turn a javascript: url into a link", () => {
    const out = html("[tap me](javascript:alert(1))");
    expect(out).not.toMatch(/href\s*=\s*["']javascript:/i);
  });

  it("opens a link through the OS, never by navigating the webview", async () => {
    // A bare <a href> inside a Tauri webview navigates the application
    // itself to the URL — the window stops being Claudepot and there is
    // no back button.
    opened.mockClear();
    render(<TranscriptMarkdown body="see [the docs](https://example.com/x)" />);
    const link = screen.getByText("the docs");
    expect(link.closest("a")).toBeNull();
    await userEvent.click(link);
    expect(opened).toHaveBeenCalledWith("https://example.com/x");
  });

  it("renders an image as its alt text, never as an image", () => {
    const out = html("![a diagram](https://evil.example/pixel.png)");
    expect(out).not.toMatch(/<img/i);
    expect(out).toContain("a diagram");
    // The source stays reachable as a tooltip, so nothing is hidden.
    expect(out).toContain('title="https://evil.example/pixel.png"');
  });

  it("leaves plain prose alone", () => {
    const out = html("Clean. The two pgrep hits are false positives.");
    expect(out).toContain("Clean. The two pgrep hits are false positives.");
    expect(out).not.toContain("<strong>");
  });

  it("the escaping assertions are capable of failing", () => {
    // A guard nobody has watched fail is indistinguishable from one that
    // cannot. Mount the markup the check above must reject, through the
    // same helper, and confirm it is rejected.
    const { container } = render(
      // The handler body is `1`, not something that throws. jsdom
      // genuinely *runs* an `onload` on a mounted iframe — which is the
      // proof this sample is live, and also why a first draft using
      // `onload="x"` ended the suite with an uncaught ReferenceError.
      <div dangerouslySetInnerHTML={{ __html: '<iframe onload="1"></iframe>' }} />,
    );
    const els = Array.from(container.querySelectorAll("*"));
    const tags = els.map((e) => e.tagName.toLowerCase());
    const handlers = els.flatMap((e) =>
      Array.from(e.attributes)
        .map((a) => a.name)
        .filter((n) => n.startsWith("on")),
    );
    expect(tags).toContain("iframe");
    expect(handlers).not.toEqual([]);
  });

  it("hands a mermaid fence to the diagram renderer", () => {
    // A diagram in an answer *is* the answer; showing its source is
    // showing the wrong artifact. `MermaidBlock` lazy-imports mermaid,
    // so this asserts the dispatch — the container is present and the
    // source is not sitting in a <pre>.
    const { container } = render(
      <TranscriptMarkdown body={"```mermaid\nflowchart TD\n  A --> B\n```"} />,
    );
    expect(container.querySelector("pre")).toBeNull();
    expect(container.textContent).not.toContain("flowchart TD");
  });

  it("leaves a non-mermaid fence as a code block", () => {
    // The dispatch must be on the language, not on "is a fence".
    const { container } = render(
      <TranscriptMarkdown body={"```rust\nlet x = 1;\n```"} />,
    );
    expect(container.querySelector("pre")).not.toBeNull();
    expect(container.textContent).toContain("let x = 1;");
  });

  it("does not treat a leading --- as frontmatter", () => {
    // The difference from config/MarkdownRenderer, which splits a
    // leading fence into a metadata card. In a message, `---` is a
    // horizontal rule the model typed.
    const out = html("---\n\nafter the rule");
    expect(out).toContain("<hr");
    expect(out).toContain("after the rule");
  });
});
