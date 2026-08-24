import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const configEffectiveMcpSpy = vi.fn();
vi.mock("../../api", () => ({
  api: { configEffectiveMcp: (...a: unknown[]) => configEffectiveMcpSpy(...a) },
}));

import { EffectiveMcpRenderer } from "./EffectiveMcpRenderer";

describe("EffectiveMcpRenderer", () => {
  it("starts in Interactive mode and re-requests when mode changes", async () => {
    configEffectiveMcpSpy.mockResolvedValue({
      enterprise_lockout: false,
      problems: [],
      servers: [],
    });
    render(<EffectiveMcpRenderer cwd="/" />);
    await waitFor(() => {
      expect(configEffectiveMcpSpy).toHaveBeenCalledWith("interactive", "/");
    });

    const niBtn = screen.getByRole("tab", { name: /non-interactive/i });
    await userEvent.click(niBtn);
    await waitFor(() => {
      expect(configEffectiveMcpSpy).toHaveBeenCalledWith(
        "non_interactive",
        "/",
      );
    });
  });

  it("renders enterprise lockout banner when flag set", async () => {
    configEffectiveMcpSpy.mockResolvedValue({
      enterprise_lockout: true,
      problems: [],
      servers: [
        {
          name: "e",
          source_scope: "policy:managed_file_composite",
          contributors: ["policy:managed_file_composite"],
          approval: "rejected",
          approval_reason: null,
          blocked_by: "enterprise_lockout",
          masked: { command: "x" },
        },
      ],
    });
    render(<EffectiveMcpRenderer cwd="/" />);
    await waitFor(() =>
      expect(
        screen.getByText(/Enterprise policy in effect/i),
      ).toBeInTheDocument(),
    );
  });

  it("renders an approval badge per server row", async () => {
    configEffectiveMcpSpy.mockResolvedValue({
      enterprise_lockout: false,
      problems: [],
      servers: [
        {
          name: "foo",
          source_scope: "project",
          contributors: ["project"],
          approval: "pending",
          approval_reason: null,
          blocked_by: null,
          masked: { command: "run-foo" },
        },
      ],
    });
    render(<EffectiveMcpRenderer cwd="/" />);
    await waitFor(() => {
      expect(screen.getByText("foo")).toBeInTheDocument();
      expect(screen.getByText("pending")).toBeInTheDocument();
    });
  });

  it("server row expands via keyboard (Enter) and exposes aria-expanded", async () => {
    configEffectiveMcpSpy.mockResolvedValue({
      enterprise_lockout: false,
      problems: [],
      servers: [
        {
          name: "foo",
          source_scope: "project",
          contributors: ["project"],
          approval: "pending",
          approval_reason: null,
          blocked_by: null,
          masked: { command: "run-foo" },
        },
      ],
    });
    render(<EffectiveMcpRenderer cwd="/" />);
    const row = await screen.findByRole("button", { expanded: false });
    expect(row).toHaveAttribute("tabIndex", "0");

    row.focus();
    await userEvent.keyboard("{Enter}");
    expect(row).toHaveAttribute("aria-expanded", "true");
    // The expanded detail row carries the masked JSON.
    expect(screen.getByText(/"command": "run-foo"/)).toBeInTheDocument();

    // Space toggles it closed again.
    await userEvent.keyboard(" ");
    expect(row).toHaveAttribute("aria-expanded", "false");
  });

  // #85: a config file that failed to load must not render as "no MCP
  // servers". CC had the same bug on `claude mcp list` and fixed it in
  // 2.1.144; the empty state below is the confident wrong answer this
  // banner replaces.
  it("names a broken config file instead of reporting an empty list", async () => {
    configEffectiveMcpSpy.mockResolvedValue({
      enterprise_lockout: false,
      servers: [],
      problems: [
        {
          path: "/proj/.mcp.json",
          kind: "malformed_json",
          detail: "expected value at line 1 column 22",
        },
      ],
    });
    render(<EffectiveMcpRenderer cwd="/proj" />);

    // The failure is stated...
    expect(await screen.findByText(/isn't valid JSON/i)).toBeInTheDocument();
    // ...against the file it happened to, in full, and copyable
    // (.claude/rules/path-display.md state C).
    expect(screen.getByText("/proj/.mcp.json")).toBeInTheDocument();
    expect(screen.getByText(/line 1 column 22/)).toBeInTheDocument();
  });

  it("treats a VS Code `servers` key as a hint, not a failure", async () => {
    configEffectiveMcpSpy.mockResolvedValue({
      enterprise_lockout: false,
      servers: [],
      problems: [
        {
          path: "/proj/.mcp.json",
          kind: "missing_servers_key",
          detail: "found servers instead",
        },
      ],
    });
    render(<EffectiveMcpRenderer cwd="/proj" />);

    // CC reads this file as zero servers without complaint, so the
    // wording points at the key rather than claiming a failure.
    const msg = await screen.findByText(/no .mcpServers. key/i);
    expect(msg).toBeInTheDocument();
    expect(msg.textContent).not.toMatch(/isn't valid JSON/i);
  });

  it("shows no banner at all when every config file loaded", async () => {
    configEffectiveMcpSpy.mockResolvedValue({
      enterprise_lockout: false,
      servers: [],
      problems: [],
    });
    render(<EffectiveMcpRenderer cwd="/proj" />);
    // The ordinary case must stay quiet — a pane that warns about
    // every project without an `.mcp.json` is worse than silence.
    expect(await screen.findByText(/No MCP servers configured/i)).toBeInTheDocument();
    expect(screen.queryByRole("status")).toBeNull();
  });
});
