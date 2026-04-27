import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { CodeRenderer } from "./CodeRenderer";

describe("CodeRenderer", () => {
  it("highlights bash via .sh extension", () => {
    const { container } = render(
      <CodeRenderer body={'echo "hi"'} path="/x/statusline.sh" />,
    );
    const code = container.querySelector("pre code");
    expect(code).toBeTruthy();
    expect(code?.className).toContain("language-bash");
    // hljs ran — at least one hljs-* span is present.
    expect(code?.querySelector("[class^='hljs-']")).toBeTruthy();
  });

  it("highlights python via shebang when extension is missing", () => {
    const body = "#!/usr/bin/env python3\nprint('hi')\n";
    const { container } = render(
      <CodeRenderer body={body} path="/x/hook" />,
    );
    const code = container.querySelector("pre code");
    expect(code?.className).toContain("language-python");
  });

  it("uses defaultLang only as a last-resort fallback", () => {
    // No extension, no shebang — defaultLang kicks in.
    const { container } = render(
      <CodeRenderer body={'console.log("x")'} path="/x/script" defaultLang="javascript" />,
    );
    expect(container.querySelector("pre code")?.className).toContain(
      "language-javascript",
    );
  });

  it("file extension wins over defaultLang (no kind override)", () => {
    // A statusline.py file must NOT be force-highlighted as bash by
    // the kind hint — the .py extension is authoritative.
    const { container } = render(
      <CodeRenderer body={"print('hi')"} path="/x/statusline.py" defaultLang="bash" />,
    );
    expect(container.querySelector("pre code")?.className).toContain(
      "language-python",
    );
  });

  it("shebang wins over defaultLang", () => {
    const body = "#!/usr/bin/env python3\nprint('hi')\n";
    const { container } = render(
      <CodeRenderer body={body} path="/x/statusline" defaultLang="bash" />,
    );
    expect(container.querySelector("pre code")?.className).toContain(
      "language-python",
    );
  });

  it("handles `env -S` shebangs and ts-node-style aliases", () => {
    const body = "#!/usr/bin/env -S python3 -u\nprint('hi')\n";
    const { container } = render(<CodeRenderer body={body} />);
    expect(container.querySelector("pre code")?.className).toContain(
      "language-python",
    );
  });

  it("falls back to auto-detect when no hint matches", () => {
    const body = '{"key": "value", "n": 42}';
    const { container } = render(<CodeRenderer body={body} />);
    const code = container.querySelector("pre code");
    expect(code).toBeTruthy();
    // Auto-detection runs even without a hint; the wrapper still
    // gets the .hljs class.
    expect(code?.className).toContain("hljs");
  });
});
