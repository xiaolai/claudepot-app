/**
 * Verifies the Settings → MCP pane's install-result block:
 *   - the "Wrote (scope): path" confirmation is copyable via the
 *     `.selectable` class (base.css), NOT via inline
 *     `userSelect: "text"`, which WKWebView ignores (React emits
 *     only the unprefixed property and the global body opt-out
 *     wins via -webkit-user-select).
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const mcpHealthSpy = vi.fn();
const snippetBodySpy = vi.fn();
const installSnippetSpy = vi.fn();
const projectListSpy = vi.fn();

vi.mock("../../api/sharedMemory", () => ({
  sharedMemoryApi: {
    mcpHealth: (...a: unknown[]) => mcpHealthSpy(...a),
    snippetBody: (...a: unknown[]) => snippetBodySpy(...a),
    installSnippet: (...a: unknown[]) => installSnippetSpy(...a),
  },
}));

vi.mock("../../api/project", () => ({
  projectApi: {
    projectList: (...a: unknown[]) => projectListSpy(...a),
  },
}));

import { McpInstallerPane } from "./McpInstallerPane";

beforeEach(() => {
  mcpHealthSpy.mockResolvedValue({ tool_visible: true, tool_count: 5 });
  snippetBodySpy.mockResolvedValue("# snippet body");
  projectListSpy.mockResolvedValue([]);
  installSnippetSpy.mockResolvedValue({
    scope: "user",
    path: "/Users/me/.claude/claudepot-mcp-instructions.md",
    bytes_written: 1234,
    include_line: "@include ~/.claude/claudepot-mcp-instructions.md",
    target_files: ["~/.claude/CLAUDE.md"],
  });
});

describe("McpInstallerPane install result", () => {
  it("renders the written path inside a .selectable block", async () => {
    const user = userEvent.setup();
    render(<McpInstallerPane pushToast={vi.fn()} />);

    await user.click(
      screen.getByRole("button", { name: /Install snippet/ }),
    );

    const wrote = await screen.findByText(/Wrote \(user\):/);
    expect(wrote.textContent).toContain(
      "/Users/me/.claude/claudepot-mcp-instructions.md",
    );
    // The copy affordance: the block opts back into text selection
    // via the class, since the global body rule disables it.
    expect(wrote.closest(".selectable")).not.toBeNull();
  });

  it("never relies on inline user-select (non-functional in WKWebView)", async () => {
    const user = userEvent.setup();
    const { container } = render(<McpInstallerPane pushToast={vi.fn()} />);

    await user.click(
      screen.getByRole("button", { name: /Install snippet/ }),
    );
    await screen.findByText(/Wrote \(user\):/);

    for (const el of Array.from(container.querySelectorAll("[style]"))) {
      expect(el.getAttribute("style") ?? "").not.toMatch(/user-select/);
    }
  });
});
