import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { MarkdownRenderer } from "./MarkdownRenderer";

describe("MarkdownRenderer", () => {
  it("renders a GFM table as a real <table>", () => {
    const md = [
      "| col a | col b |",
      "| ----- | ----- |",
      "| 1     | apple |",
      "| 2     | pear  |",
    ].join("\n");
    const { container } = render(<MarkdownRenderer body={md} />);
    const table = container.querySelector("table");
    expect(table).toBeTruthy();
    expect(table?.querySelectorAll("tbody tr").length).toBe(2);
    // Header cell is a <th>, not a <td> — confirms GFM table parser
    // ran instead of falling back to a paragraph.
    expect(container.querySelectorAll("thead th").length).toBe(2);
  });

  it("highlights fenced code blocks (rehype-highlight wires hljs classes)", () => {
    const md = "```bash\necho hi\n```";
    const { container } = render(<MarkdownRenderer body={md} />);
    const code = container.querySelector("pre code");
    expect(code).toBeTruthy();
    // hljs classes go on the <code>; the language hint becomes a
    // class. Either form proves the highlighter ran on the tree.
    expect(code?.className).toMatch(/language-bash|hljs/);
    // hljs-* spans appear inside when grammar runs successfully.
    expect(code?.querySelector(".hljs-built_in, .hljs-keyword, .hljs-string"))
      .toBeTruthy();
  });

  it("extracts YAML frontmatter into the standalone aside", () => {
    const md = ["---", "name: test-agent", "model: opus", "---", "", "Body text."].join("\n");
    const { container } = render(<MarkdownRenderer body={md} />);
    const aside = container.querySelector('aside[aria-label="Frontmatter"]');
    expect(aside).toBeTruthy();
    expect(aside?.textContent).toContain("name");
    expect(aside?.textContent).toContain("test-agent");
    // Body still renders below the aside.
    expect(container.querySelector("p")?.textContent).toBe("Body text.");
  });

  it("renders task lists with checkbox inputs", () => {
    const md = ["- [x] done", "- [ ] todo"].join("\n");
    const { container } = render(<MarkdownRenderer body={md} />);
    const checkboxes = container.querySelectorAll('input[type="checkbox"]');
    expect(checkboxes.length).toBe(2);
    expect((checkboxes[0] as HTMLInputElement).checked).toBe(true);
    expect((checkboxes[1] as HTMLInputElement).checked).toBe(false);
  });

  it("does not render embedded HTML as markup (escape-only)", () => {
    const md = 'Hello <script>alert("xss")</script> world';
    const { container } = render(<MarkdownRenderer body={md} />);
    // No <script> element should reach the DOM.
    expect(container.querySelector("script")).toBeNull();
    // Text content keeps the literal string.
    expect(container.textContent).toContain("alert");
  });

  it("renders markdown links as inert spans (no <a href>)", () => {
    const md = "See [the docs](https://example.com/docs) for details.";
    const { container } = render(<MarkdownRenderer body={md} />);
    // No anchor — Tauri webview would otherwise navigate away.
    expect(container.querySelector("a")).toBeNull();
    // The destination is preserved in a tooltip for copy-paste.
    const link = container.querySelector('span.md-link');
    expect(link).toBeTruthy();
    expect(link?.getAttribute("title")).toBe("https://example.com/docs");
    expect(link?.textContent).toBe("the docs");
  });

  it("never emits a real <img> for markdown image syntax", () => {
    const md = "![logo](https://malicious.example.com/track.png)";
    const { container } = render(<MarkdownRenderer body={md} />);
    // Critical: no real <img> means no network fetch on render.
    expect(container.querySelector("img")).toBeNull();
    // Replacement carries the alt + host so the user sees what was
    // suppressed without leaking pixels to the URL.
    const ph = container.querySelector('[role="img"]');
    expect(ph).toBeTruthy();
    expect(ph?.getAttribute("aria-label")).toBe("logo");
    expect(ph?.textContent).toContain("logo");
    expect(ph?.textContent).toContain("malicious.example.com");
  });
});
