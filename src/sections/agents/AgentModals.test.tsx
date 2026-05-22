import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// The api module is a barrel of Tauri `invoke` wrappers. Mock the
// two surfaces ReviewInstallModal touches; everything else is unused
// in these tests.
const agentsGet = vi.fn();
const agentInstall = vi.fn();
vi.mock("../../api", () => ({
  api: {
    agentsGet: (...a: unknown[]) => agentsGet(...a),
    agentInstall: (...a: unknown[]) => agentInstall(...a),
  },
}));

// AgentModals pulls `pushToast` off the app-state provider.
const pushToast = vi.fn();
vi.mock("../../providers/AppStateProvider", () => ({
  useAppState: () => ({ pushToast }),
}));

import { ReviewInstallModal } from "./AgentModals";
import type { AgentDetailsDto, AgentSummaryDto } from "../../types";

const summary = (overrides?: Partial<AgentSummaryDto>): AgentSummaryDto => ({
  id: "agent-1",
  name: "evening-digest",
  display_name: null,
  description: null,
  enabled: true,
  binary_kind: "first_party",
  binary_route_id: null,
  model: "sonnet",
  cwd: "/Users/me/project",
  permission_mode: "dontAsk",
  allowed_tools: ["Read", "Grep"],
  max_budget_usd: 0.5,
  trigger_kind: "cron",
  cron: "0 9 * * *",
  timezone: null,
  lifecycle: "draft",
  created_at: "2026-05-22T00:00:00Z",
  updated_at: "2026-05-22T00:00:00Z",
  ...overrides,
});

const details = (overrides?: Partial<AgentDetailsDto>): AgentDetailsDto => ({
  summary: summary(),
  prompt: "summarize today's commits",
  system_prompt: null,
  append_system_prompt: null,
  add_dir: [],
  fallback_model: null,
  output_format: "json",
  json_schema: null,
  bare: false,
  extra_env: {},
  platform_options: {
    wake_to_run: false,
    catch_up_if_missed: true,
    run_when_logged_out: false,
  },
  log_retention_runs: 50,
  disallowed_tools: [],
  mcp_servers: [],
  run_as: null,
  task_budget: null,
  rate_limit: null,
  drafted_by: "claude-code@2026-05-22",
  created_via: "cli_draft",
  ...overrides,
});

describe("ReviewInstallModal — the human-in-the-loop install gate", () => {
  beforeEach(() => {
    agentsGet.mockReset();
    agentInstall.mockReset();
    pushToast.mockReset();
  });

  it("renders nothing when closed", () => {
    render(
      <ReviewInstallModal
        open={false}
        target={summary()}
        onClose={() => {}}
        onInstalled={() => {}}
      />,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("keeps Install disabled until the review pane has loaded", async () => {
    // Hold the details fetch open so we can observe the pre-load
    // state: the review pane is absent and Install is disabled.
    let resolveDetails!: (d: AgentDetailsDto) => void;
    agentsGet.mockReturnValue(
      new Promise<AgentDetailsDto>((res) => {
        resolveDetails = res;
      }),
    );

    render(
      <ReviewInstallModal
        open
        target={summary()}
        onClose={() => {}}
        onInstalled={() => {}}
      />,
    );

    // Before the spec arrives: the loading state shows, the review
    // grid (the prompt the human consents to) is NOT yet rendered,
    // and the Install button is disabled.
    expect(screen.getByText("Loading…")).toBeInTheDocument();
    expect(
      screen.queryByText("summarize today's commits"),
    ).not.toBeInTheDocument();
    const installBefore = screen.getByRole("button", { name: "Install" });
    expect(installBefore).toBeDisabled();

    // The spec arrives — the review pane renders and only THEN does
    // Install enable.
    resolveDetails(details());
    await waitFor(() =>
      expect(
        screen.getByText("summarize today's commits"),
      ).toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: "Install" })).toBeEnabled();
  });

  it("flags a non-GUI provenance (F19) — cli_draft renders the AI-drafted banner", async () => {
    agentsGet.mockResolvedValue(details({ created_via: "cli_draft" }));
    render(
      <ReviewInstallModal
        open
        target={summary()}
        onClose={() => {}}
        onInstalled={() => {}}
      />,
    );
    await waitFor(() =>
      expect(
        screen.getByText("summarize today's commits"),
      ).toBeInTheDocument(),
    );
    // The provenance banner copy: it must call out that the
    // record is not GUI-authored and steer scrutiny to the spec.
    expect(screen.getByText(/AI-drafted/)).toBeInTheDocument();
    expect(
      screen.getByText(/Not authored in the GUI/),
    ).toBeInTheDocument();
  });

  it("does NOT flag provenance when created_via == 'gui'", async () => {
    agentsGet.mockResolvedValue(details({ created_via: "gui" }));
    render(
      <ReviewInstallModal
        open
        target={summary()}
        onClose={() => {}}
        onInstalled={() => {}}
      />,
    );
    await waitFor(() =>
      expect(
        screen.getByText("summarize today's commits"),
      ).toBeInTheDocument(),
    );
    expect(screen.queryByText(/AI-drafted/)).toBeNull();
    expect(
      screen.queryByText(/Not authored in the GUI/),
    ).toBeNull();
  });

  it("warns about the recorded-but-ignored run_as (F22)", async () => {
    agentsGet.mockResolvedValue(
      details({ run_as: "dev@example.com" }),
    );
    render(
      <ReviewInstallModal
        open
        target={summary()}
        onClose={() => {}}
        onInstalled={() => {}}
      />,
    );
    await waitFor(() =>
      expect(
        screen.getByText("summarize today's commits"),
      ).toBeInTheDocument(),
    );
    // The pinned email renders verbatim.
    expect(
      screen.getByText("dev@example.com"),
    ).toBeInTheDocument();
    // And the inline "not yet wired" note must accompany it so the
    // user is not surprised the run uses a different account.
    expect(
      screen.getByText(/Per-run credential injection is not yet wired/),
    ).toBeInTheDocument();
  });

  it("does NOT show the run_as warning when run_as is null", async () => {
    agentsGet.mockResolvedValue(details({ run_as: null }));
    render(
      <ReviewInstallModal
        open
        target={summary()}
        onClose={() => {}}
        onInstalled={() => {}}
      />,
    );
    await waitFor(() =>
      expect(
        screen.getByText("summarize today's commits"),
      ).toBeInTheDocument(),
    );
    expect(
      screen.queryByText(/Per-run credential injection is not yet wired/),
    ).toBeNull();
    expect(
      screen.getByText("active account at fire time"),
    ).toBeInTheDocument();
  });

  it("flags a bypassPermissions draft before the human installs it", async () => {
    agentsGet.mockResolvedValue(
      details({
        // Use `gui` so the F19 origin-warning alert is suppressed
        // and this test stays focused on the bypassPermissions
        // danger alert specifically.
        created_via: "gui",
        summary: summary({ permission_mode: "bypassPermissions" }),
      }),
    );
    render(
      <ReviewInstallModal
        open
        target={summary({ permission_mode: "bypassPermissions" })}
        onClose={() => {}}
        onInstalled={() => {}}
      />,
    );
    // The danger alert must render in the review pane.
    await waitFor(() =>
      expect(screen.getByRole("alert")).toBeInTheDocument(),
    );
    expect(
      screen.getAllByText("bypassPermissions").length,
    ).toBeGreaterThan(0);
  });

  it("calls agent_install only after the human clicks Install", async () => {
    agentsGet.mockResolvedValue(details());
    agentInstall.mockResolvedValue(summary({ lifecycle: "installed" }));
    const onInstalled = vi.fn();
    const user = userEvent.setup();

    render(
      <ReviewInstallModal
        open
        target={summary()}
        onClose={() => {}}
        onInstalled={onInstalled}
      />,
    );

    // Review pane must be on screen first.
    await waitFor(() =>
      expect(
        screen.getByText("summarize today's commits"),
      ).toBeInTheDocument(),
    );
    // The gate has not fired yet — no install call.
    expect(agentInstall).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Install" }));
    await waitFor(() =>
      expect(agentInstall).toHaveBeenCalledWith("agent-1"),
    );
    expect(onInstalled).toHaveBeenCalledTimes(1);
  });

  it("surfaces an install failure as a toast and does not close", async () => {
    agentsGet.mockResolvedValue(details());
    agentInstall.mockRejectedValue(new Error("register failed"));
    const onClose = vi.fn();
    const user = userEvent.setup();

    render(
      <ReviewInstallModal
        open
        target={summary()}
        onClose={onClose}
        onInstalled={() => {}}
      />,
    );
    await waitFor(() =>
      expect(
        screen.getByText("summarize today's commits"),
      ).toBeInTheDocument(),
    );
    await user.click(screen.getByRole("button", { name: "Install" }));
    await waitFor(() =>
      expect(pushToast).toHaveBeenCalledWith(
        "error",
        expect.stringContaining("register failed"),
      ),
    );
    expect(onClose).not.toHaveBeenCalled();
  });
});
