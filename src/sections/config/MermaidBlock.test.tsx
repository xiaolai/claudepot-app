import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, waitFor } from "@testing-library/react";
import { MarkdownRenderer } from "./MarkdownRenderer";
import tauriConf from "../../../src-tauri/tauri.conf.json";

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

/**
 * Mermaid styles its SVG through a `<style>` element it inserts at
 * render time, and that element is governed by the webview's
 * `style-src`. The configured policy grants `'unsafe-inline'`, but Tauri
 * hardens the embedded bundle on its own: it stamps a nonce on
 * `index.html`'s inline `<style>` and appends `'nonce-…'` to `style-src`
 * (tauri-codegen `map_core_assets` → `inject_nonce_token`; runtime
 * `replace_csp_nonce`). A nonce or hash source makes a browser IGNORE
 * `'unsafe-inline'` (CSP Level 3, "does a source list allow all inline
 * behavior"), so in the release app — and only there — mermaid's
 * stylesheet was refused and every diagram drew with SVG defaults: black
 * node fills, edge paths filled black, labels anchored `start` and
 * spilling out of the right edge of their boxes. Dev never showed it,
 * because the nonce is injected only into embedded assets, and the
 * remote panel never did, because its server writes its own header.
 * Reproduced in Playwright WebKit by adding one `'nonce-…'` to a
 * harness's `style-src`; the same page without it drew correctly.
 *
 * `dangerousDisableAssetCspModification: ["style-src"]` keeps the
 * written policy in force for styles while leaving `script-src` under
 * Tauri's nonce/hash hardening. Both halves are asserted here because
 * either one without the other is a silent no-op in release — and
 * nothing in CI runs the release webview.
 */
describe("the release CSP admits mermaid's runtime <style>", () => {
  const security = tauriConf.app.security;
  // Read as `unknown` so a config that drops the key fails the assertion
  // below with the key's name in the message, instead of failing to
  // compile on a property access.
  const disabled: unknown = (security as Record<string, unknown>)
    .dangerousDisableAssetCspModification;
  const directives = new Map(
    security.csp.split(";").map((d) => {
      const [name, ...sources] = d.trim().split(/\s+/);
      return [name, sources] as const;
    }),
  );

  it("style-src grants 'unsafe-inline'", () => {
    expect(directives.get("style-src")).toContain("'unsafe-inline'");
  });

  it("Tauri is told not to append nonce/hash sources to style-src", () => {
    // A nonce or hash source in the list would make the grant above
    // ignored, which is exactly the state that shipped.
    expect(
      disabled,
      "app.security.dangerousDisableAssetCspModification must list style-src",
    ).toEqual(expect.arrayContaining(["style-src"]));
  });

  it("script-src stays under Tauri's hardening", () => {
    // `true` disables the injection for every directive, scripts included.
    expect(disabled).not.toBe(true);
    expect(disabled).not.toEqual(expect.arrayContaining(["script-src"]));
  });
});
