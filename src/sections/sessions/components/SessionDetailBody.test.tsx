import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";

import { MetaMatchNote } from "./SessionDetailBody";

/**
 * DOM-level verification for the meta-match note.
 *
 * The classifier (`classifyMetaMatch`) intentionally preserves the
 * raw value — redaction is the renderer's job. These tests pin the
 * renderer contract: whatever sk-ant-shaped substring the classifier
 * hands in must be masked before it lands in the DOM.
 *
 * We assert against `container.textContent` for the rendered text
 * nodes; a dedicated `getAttribute("title")` check covers the one
 * attribute path the component writes (the `<code title={safeValue}>`
 * tooltip). `textContent` does NOT walk attributes, so if a future
 * refactor adds more attributes carrying user-controlled strings,
 * extend the test accordingly.
 */
describe("MetaMatchNote redaction", () => {
  const LEAKED = "sk-ant-oat01-AbcdEfGh1234XYZ";

  it("redacts sk-ant-* in a matched project path (text + title attr)", () => {
    const { container } = render(
      <MetaMatchNote
        query="tauri"
        matches={[
          { field: "project path", value: `/tmp/${LEAKED}/src-tauri` },
        ]}
      />,
    );
    expect(container.textContent).not.toContain(LEAKED);
    // `sk-ant-` is part of the redaction marker `sk-ant-***…`, but
    // the raw suffix after it must not survive.
    expect(container.textContent).not.toContain("AbcdEfGh1234");
    expect(container.textContent).toMatch(/sk-ant-\*+/);
    // Component also stores the value in `<code title=…>` for
    // hover-to-see-full-path. Verify that attribute is redacted too,
    // since `textContent` above does not walk attributes.
    const code = container.querySelector("code");
    expect(code).not.toBeNull();
    expect(code?.getAttribute("title")).not.toContain(LEAKED);
    expect(code?.getAttribute("title")).toMatch(/sk-ant-\*+/);
  });

  it("redacts sk-ant-* in a branch name", () => {
    const { container } = render(
      <MetaMatchNote
        query="leak"
        matches={[{ field: "branch", value: `leak-${LEAKED}` }]}
      />,
    );
    expect(container.textContent).not.toContain(LEAKED);
  });

  it("redacts sk-ant-* in the user's own query text", () => {
    // The user might paste a token into the search box itself. The
    // banner interpolates the query into its opening sentence; that
    // path must be redacted too.
    const { container } = render(
      <MetaMatchNote
        query={LEAKED}
        matches={[{ field: "project path", value: "/tmp/harmless" }]}
      />,
    );
    expect(container.textContent).not.toContain(LEAKED);
  });

  it("renders nothing when there are zero matches", () => {
    // Defensive: a caller that hands in an empty array must not
    // produce a dangling empty note.
    const { container } = render(<MetaMatchNote query="x" matches={[]} />);
    expect(container.firstChild).toBeNull();
  });

  it("preserves non-secret free-text verbatim", () => {
    const { container } = render(
      <MetaMatchNote
        query="tauri"
        matches={[
          { field: "project path", value: "/Users/joker/src-tauri/app" },
          { field: "branch", value: "feat/tauri-upgrade" },
        ]}
      />,
    );
    expect(container.textContent).toContain("/Users/joker/src-tauri/app");
    expect(container.textContent).toContain("feat/tauri-upgrade");
  });
});
