import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// AgentForm + its CronInput child both call into the api barrel.
// Name validation and cron validation must resolve "valid" so the
// happy-path submit gate opens; tests override per-case as needed.
const agentsValidateName = vi.fn();
const agentsValidateCron = vi.fn();
vi.mock("../../api", () => ({
  api: {
    agentsValidateName: (...a: unknown[]) => agentsValidateName(...a),
    agentsValidateCron: (...a: unknown[]) => agentsValidateCron(...a),
  },
}));

import { AgentForm } from "./AgentForm";
import type { AgentCreateDto, SchedulerCapabilitiesDto } from "../../types";

const caps: SchedulerCapabilitiesDto = {
  wake_to_run: false,
  catch_up_if_missed: true,
  run_when_logged_out: false,
  native_label: "launchd",
  artifact_dir: null,
};

/** Fill the minimum fields so `canSubmit` can open, then return the
 *  current "Create" button. Leaves rate-limit / task-budget empty so
 *  each test drives those sentinels. */
async function fillRequiredFields(user: ReturnType<typeof userEvent.setup>) {
  await user.type(
    screen.getByPlaceholderText("morning-pr-summary"),
    "nightly-digest",
  );
  await user.type(
    screen.getByPlaceholderText("/Users/me/github/myproject"),
    "/Users/me/project",
  );
  await user.type(
    screen.getByPlaceholderText("summarize today's PRs..."),
    "do the thing",
  );
}

describe("AgentForm — clear-sentinel conversions + submit gating", () => {
  beforeEach(() => {
    agentsValidateName.mockReset();
    agentsValidateCron.mockReset();
    agentsValidateName.mockResolvedValue({
      valid: true,
      error: null,
      already_taken: false,
    });
    agentsValidateCron.mockResolvedValue({
      valid: true,
      error: null,
      next_runs: [],
    });
  });

  it("maps empty task-budget and empty rate-limit fields to null in the DTO", async () => {
    // The clear sentinels: an empty task-budget input and two empty
    // rate-limit inputs must arrive at the DTO as `null`, never `0`
    // and never an all-null RateLimit object.
    const onSubmit = vi.fn<(dto: AgentCreateDto) => void>();
    const user = userEvent.setup();
    render(
      <AgentForm
        routes={[]}
        capabilities={caps}
        busy={false}
        submitLabel="Create"
        onSubmit={onSubmit}
        onCancel={() => {}}
      />,
    );
    await fillRequiredFields(user);
    // Default permission mode is bypassPermissions with a default
    // allowed-tools list, so the submit gate is otherwise open.
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Create" })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "Create" }));

    expect(onSubmit).toHaveBeenCalledTimes(1);
    const dto = onSubmit.mock.calls[0][0];
    expect(dto.task_budget).toBeNull();
    expect(dto.rate_limit).toBeNull();
  });

  it("emits a populated rate_limit when a field is filled, dropping the empty side", async () => {
    const onSubmit = vi.fn<(dto: AgentCreateDto) => void>();
    const user = userEvent.setup();
    render(
      <AgentForm
        routes={[]}
        capabilities={caps}
        busy={false}
        submitLabel="Create"
        onSubmit={onSubmit}
        onCancel={() => {}}
      />,
    );
    await fillRequiredFields(user);
    // Fill only "max runs per day"; "min interval" stays empty.
    await user.type(screen.getByPlaceholderText("24"), "12");
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Create" })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "Create" }));

    const dto = onSubmit.mock.calls[0][0];
    expect(dto.rate_limit).not.toBeNull();
    expect(dto.rate_limit?.max_per_day).toBe(12);
    // The untouched side is null, not 0.
    expect(dto.rate_limit?.min_interval_secs).toBeNull();
  });

  it("blocks submit while a rate-limit field holds an invalid value", async () => {
    const onSubmit = vi.fn();
    const user = userEvent.setup();
    render(
      <AgentForm
        routes={[]}
        capabilities={caps}
        busy={false}
        submitLabel="Create"
        onSubmit={onSubmit}
        onCancel={() => {}}
      />,
    );
    await fillRequiredFields(user);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Create" })).toBeEnabled(),
    );
    // A zero rate limit is invalid (a populated field must be a
    // positive integer) — the submit gate must close.
    await user.type(screen.getByPlaceholderText("3600"), "0");
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Create" }),
      ).toBeDisabled(),
    );
    expect(
      screen.getByText(/Minimum interval must be a positive whole number/),
    ).toBeInTheDocument();
    // The submit handler never fired.
    await user.click(screen.getByRole("button", { name: "Create" }));
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("blocks submit when bypassPermissions has an empty allowed-tools list", async () => {
    // bypassPermissions without a whitelist is the cross-field
    // invariant the form must refuse before the DTO crosses IPC.
    const onSubmit = vi.fn();
    const user = userEvent.setup();
    render(
      <AgentForm
        routes={[]}
        capabilities={caps}
        busy={false}
        submitLabel="Create"
        onSubmit={onSubmit}
        onCancel={() => {}}
      />,
    );
    await fillRequiredFields(user);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Create" })).toBeEnabled(),
    );
    // Clear the default allowed-tools list.
    await user.clear(
      screen.getByPlaceholderText("Read Grep Glob Bash(git *)"),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Create" }),
      ).toBeDisabled(),
    );
    expect(
      screen.getByText(/bypassPermissions requires a non-empty whitelist/),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Create" }));
    expect(onSubmit).not.toHaveBeenCalled();
  });
});
