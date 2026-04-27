import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, waitFor } from "@testing-library/react";
import { MarkdownRenderer } from "./MarkdownRenderer";

// Stand in for the mermaid runtime. The real library needs real
// SVG measurement (getBBox), which jsdom does not implement, so we
// stub it. Mocking by module specifier lets the dynamic import in
// MermaidBlock resolve to this object without ever loading the
// actual ~600 KB chunk.
vi.mock("mermaid", () => ({
  default: {
    initialize: vi.fn(),
    render: vi.fn().mockResolvedValue({
      svg: '<svg xmlns="http://www.w3.org/2000/svg" data-test="mocked"/>',
    }),
  },
}));

describe("MermaidBlock via MarkdownRenderer", () => {
  // Reset the module-level mock state so tests that inspect call
  // history aren't order-coupled to earlier renders.
  beforeEach(async () => {
    const mermaid = (await import("mermaid")).default;
    (mermaid.initialize as ReturnType<typeof vi.fn>).mockClear();
    (mermaid.render as ReturnType<typeof vi.fn>).mockClear();
  });

  it("routes a language-mermaid fence to a MermaidBlock container", async () => {
    const md = "```mermaid\ngraph TD\n  A --> B\n```";
    const { container } = render(<MarkdownRenderer body={md} />);

    // The placeholder mounts synchronously with role=img + aria-label.
    const block = container.querySelector(
      '.mermaid-block[role="img"][aria-label="Mermaid diagram"]',
    );
    expect(block).toBeTruthy();

    // After the (mocked) dynamic import resolves, the SVG is injected.
    await waitFor(() => {
      expect(block?.querySelector('svg[data-test="mocked"]')).toBeTruthy();
    });
  });

  it("does not route a non-mermaid fence to MermaidBlock", () => {
    const md = "```bash\necho hi\n```";
    const { container } = render(<MarkdownRenderer body={md} />);
    expect(container.querySelector(".mermaid-block")).toBeNull();
    // Bash fence still renders as a highlighted <pre><code>.
    expect(container.querySelector("pre code")).toBeTruthy();
  });

  it("strips <script>, on* handlers, and javascript: hrefs from the SVG", async () => {
    const malicious = [
      '<svg xmlns="http://www.w3.org/2000/svg"',
      '     xmlns:xlink="http://www.w3.org/1999/xlink"',
      '     onload="alert(1)">',
      '  <script>window.__pwn=1</script>',
      '  <foreignObject><iframe src="javascript:alert(1)"/></foreignObject>',
      '  <a xlink:href="javascript:alert(1)"><circle onclick="alert(1)" r="10"/></a>',
      '</svg>',
    ].join("\n");
    const mermaid = (await import("mermaid")).default;
    (mermaid.render as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      svg: malicious,
    });

    const md = "```mermaid\nflowchart\n  A\n```";
    const { container } = render(<MarkdownRenderer body={md} />);
    const block = container.querySelector(".mermaid-block");

    await waitFor(() => {
      expect(block?.querySelector("svg")).toBeTruthy();
    });

    const svg = block?.querySelector("svg") as SVGElement;
    expect(svg.querySelector("script")).toBeNull();
    expect(svg.querySelector("foreignObject")).toBeNull();
    expect(svg.getAttribute("onload")).toBeNull();
    const circle = svg.querySelector("circle");
    expect(circle?.getAttribute("onclick")).toBeNull();
    const link = svg.querySelector("a");
    // xlink:href with javascript: is stripped; the <a> wrapper itself
    // is allowed to remain (mermaid uses it for legitimate node links).
    expect(link?.getAttribute("xlink:href")).toBeNull();
  });

  it("scrubs javascript: hrefs on the root <svg> too (not just descendants)", async () => {
    const malicious = [
      '<svg xmlns="http://www.w3.org/2000/svg"',
      '     xmlns:xlink="http://www.w3.org/1999/xlink"',
      '     xlink:href="javascript:alert(1)"',
      '     href="javascript:alert(2)">',
      '  <circle r="10"/>',
      '</svg>',
    ].join("\n");
    const mermaid = (await import("mermaid")).default;
    (mermaid.render as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      svg: malicious,
    });

    const md = "```mermaid\nflowchart\n  X\n```";
    const { container } = render(<MarkdownRenderer body={md} />);
    const block = container.querySelector(".mermaid-block");
    await waitFor(() => {
      expect(block?.querySelector("svg")).toBeTruthy();
    });
    const svg = block?.querySelector("svg") as SVGElement;
    expect(svg.getAttribute("xlink:href")).toBeNull();
    expect(svg.getAttribute("href")).toBeNull();
  });

  it("preserves diagram source across the pipeline", async () => {
    const source = "sequenceDiagram\n  Alice->>Bob: hi";
    const md = "```mermaid\n" + source + "\n```";
    render(<MarkdownRenderer body={md} />);

    const mermaid = (await import("mermaid")).default;
    await waitFor(() => {
      expect(mermaid.render).toHaveBeenCalled();
    });
    // react-markdown appends a trailing newline to fenced-code content;
    // we don't strip it (mermaid tolerates it), so just verify the
    // diagram body survives the round-trip.
    const calls = (mermaid.render as ReturnType<typeof vi.fn>).mock.calls;
    const args = calls[calls.length - 1];
    expect(args[0]).toMatch(/^mermaid-/);
    expect(String(args[1]).trim()).toBe(source);
  });
});
