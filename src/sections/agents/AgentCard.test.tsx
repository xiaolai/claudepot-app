import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { AgentCard } from "./AgentCard";
import type { AgentRunStatus, AgentSummaryDto, InFlightRun } from "../../types";

vi.mock("./RunHistoryPanel", () => ({
  RunHistoryPanel: () => null,
}));

function agent(over: Partial<AgentSummaryDto> = {}): AgentSummaryDto {
  return {
    id: "a1",
    name: "cc-upstream-watch",
    display_name: "CC upstream watch",
    description: null,
    enabled: true,
    lifecycle: "installed",
    trigger_kind: "cron",
    trigger_summary: "0 3 1 * *",
    next_run: null,
    last_run: null,
    model: "sonnet",
    permission_mode: "dontAsk",
    allowed_tools: [],
    max_budget_usd: null,
    cwd: "/repo",
    ...over,
  } as unknown as AgentSummaryDto;
}

function run(over: Partial<InFlightRun> = {}): InFlightRun {
  return {
    agent_id: "a1",
    run_id: "r1",
    started_ms: Date.now() - 14 * 60 * 1000,
    state: "running",
    ...over,
  };
}

function status(over: Partial<AgentRunStatus> = {}): AgentRunStatus {
  return { agent_id: "a1", in_flight: null, last: null, ...over };
}

const noop = () => {};
const props = {
  mutating: false,
  runsRefreshKey: 0,
  onRun: noop,
  onEdit: noop,
  onToggle: noop,
  onRemove: noop,
  onReview: noop,
};

describe("AgentCard — run visibility", () => {
  it("shows nothing when no run is in flight", () => {
    render(<AgentCard agent={agent()} {...props} />);
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.getByRole("button", { name: /run now/i })).toBeEnabled();
  });

  // §2.2's three idle rows. "never completed a run" and "the last run
  // failed" are different statements and must not collapse.
  it("says never run when the agent has no completed run", () => {
    render(<AgentCard agent={agent()} runStatus={status()} {...props} />);
    expect(screen.getByText(/never run/i)).toBeInTheDocument();
  });

  it("reports a successful last run with when and how long", () => {
    render(
      <AgentCard
        agent={agent()}
        runStatus={status({
          last: {
            run_id: "r0",
            ended_ms: Date.now() - 2 * 60 * 60 * 1000,
            exit_code: 0,
            duration_ms: 886422,
          },
        })}
        {...props}
      />,
    );
    expect(screen.getByText(/last run 2h/i)).toBeInTheDocument();
    expect(screen.getByText(/14m/)).toBeInTheDocument();
  });

  it("reports a failed last run with its exit code and nothing invented", () => {
    render(
      <AgentCard
        agent={agent()}
        runStatus={status({
          last: {
            run_id: "r0",
            ended_ms: Date.now() - 5 * 60 * 1000,
            exit_code: 1,
            duration_ms: 19247,
          },
        })}
        {...props}
      />,
    );
    const row = screen.getByText(/last run failed/i);
    expect(row).toHaveTextContent(/exit 1/);
    // The cause is not in result.json; guessing one is how a status
    // surface starts lying.
    expect(row).not.toHaveTextContent(/budget|timeout|crash/i);
  });

  // A live run supersedes history — two ambient rows on one card would
  // breach the signal budget.
  it("hides last-run history while a run is live", () => {
    render(
      <AgentCard
        agent={agent()}
        runStatus={status({
          in_flight: run(),
          last: {
            run_id: "r0",
            ended_ms: Date.now() - 60_000,
            exit_code: 0,
            duration_ms: 1000,
          },
        })}
        {...props}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(/Running/);
    expect(screen.queryByText(/last run/i)).toBeNull();
  });

  it("shows elapsed time while a run is live", () => {
    render(<AgentCard agent={agent()} runStatus={status({ in_flight: run() })} {...props} />);
    expect(screen.getByRole("status")).toHaveTextContent(/Running/);
    expect(screen.getByRole("status")).toHaveTextContent(/14m/);
  });

  // §1.1: the button used to re-enable itself five minutes into a
  // fifteen-minute run, inviting a second concurrent run.
  it("disables Run Now while running and states the reason inline", () => {
    render(<AgentCard agent={agent()} runStatus={status({ in_flight: run() })} {...props} />);
    expect(screen.getByRole("button", { name: /run now/i })).toBeDisabled();
    // design.md: the reason is inline text, not a tooltip/title.
    expect(screen.getByText(/already running/i)).toBeInTheDocument();
  });

  // §1.2 third row, and the whole reason the agent noun exists: a
  // cron-fired run involves no click, so nothing local ever knows about
  // it. It must render exactly like a manual one.
  it("renders a cron-fired run identically to a manual one", () => {
    // No onRun was ever called; the run arrives purely from the poll.
    render(<AgentCard agent={agent()} runStatus={status({ in_flight: run() })} {...props} />);
    expect(screen.getByRole("status")).toHaveTextContent(/Running/);
    expect(screen.getByRole("button", { name: /run now/i })).toBeDisabled();
  });

  // A crashed run must read as neither "running" nor "fine".
  it("labels an interrupted run without claiming it is running", () => {
    render(
      <AgentCard agent={agent()} runStatus={status({ in_flight: run({ state: "interrupted" }) })} {...props} />,
    );
    const row = screen.getByRole("status");
    expect(row).toHaveTextContent(/Interrupted/);
    expect(row).not.toHaveTextContent(/Running/);
    // Interrupted means nothing is working on it — Run Now is available.
    expect(screen.getByRole("button", { name: /run now/i })).toBeEnabled();
  });

  // A failed poll must not render as idle — same rule as scan_incomplete.
  it("says it cannot determine state rather than showing idle", () => {
    render(<AgentCard agent={agent()} runsUnknown {...props} />);
    expect(screen.getByText(/can't determine/i)).toBeInTheDocument();
  });

  // `mutating` is a sub-second state; it locks the controls but must not
  // claim a run is in progress.
  it("a mutation locks controls without claiming a run", () => {
    render(<AgentCard agent={agent()} {...props} mutating />);
    expect(screen.getByRole("button", { name: /run now/i })).toBeDisabled();
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.queryByText(/already running/i)).toBeNull();
  });
});
