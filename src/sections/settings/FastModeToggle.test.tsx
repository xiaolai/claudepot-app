import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { FastModeState } from "../../api/fastMode";
import { FastModeToggle } from "./FastModeToggle";

const stateImpl = { fn: vi.fn() };
const setImpl = { fn: vi.fn() };
const perSessionImpl = { fn: vi.fn() };

vi.mock("../../api", () => ({
  api: {
    fastModeState: () => stateImpl.fn(),
    fastModeSet: (enabled: boolean) => setImpl.fn(enabled),
    fastModeSetPerSession: (required: boolean) => perSessionImpl.fn(required),
  },
}));

function mkState(over: Partial<FastModeState> = {}): FastModeState {
  return {
    effective: false,
    decided_by: "default",
    user_writable: true,
    user_settings_value: null,
    per_session_opt_in: false,
    env_disabled: false,
    facts: {
      models: ["claude-opus-5", "claude-opus-4-8"],
      input_per_mtok: 10,
      output_per_mtok: 50,
    },
    ...over,
  };
}

const pushToast = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  stateImpl.fn = vi.fn(() => Promise.resolve(mkState()));
  setImpl.fn = vi.fn((enabled: boolean) =>
    Promise.resolve(mkState({ effective: enabled })),
  );
  perSessionImpl.fn = vi.fn((required: boolean) =>
    Promise.resolve(mkState({ per_session_opt_in: required })),
  );
});

describe("FastModeToggle", () => {
  it("renders off by default and turns on", async () => {
    render(<FastModeToggle pushToast={pushToast} />);
    const sw = await screen.findByRole("switch", { name: "Fast mode" });
    await waitFor(() => expect(sw).toHaveAttribute("aria-checked", "false"));

    await userEvent.click(sw);
    expect(setImpl.fn).toHaveBeenCalledWith(true);
    await waitFor(() => expect(sw).toHaveAttribute("aria-checked", "true"));
  });

  it("states the cost tradeoff, using rates from the backend", async () => {
    // The rate is not hardcoded in the component — a stale "$10/$50" in
    // TSX would survive a real rate change silently.
    render(<FastModeToggle pushToast={pushToast} />);
    expect(await screen.findByText(/\$10\/\$50 per MTok/)).toBeInTheDocument();
    expect(screen.getByText(/usage credits/)).toBeInTheDocument();
  });

  it("names the supported models from the backend list", async () => {
    render(<FastModeToggle pushToast={pushToast} />);
    expect(
      await screen.findByText(/Opus 5 and Opus 4\.8/),
    ).toBeInTheDocument();
  });

  it("locks the switch and states the reason when the env var forces it off", async () => {
    stateImpl.fn = vi.fn(() =>
      Promise.resolve(
        mkState({
          effective: false,
          user_writable: false,
          env_disabled: true,
          decided_by: "env_disabled",
          user_settings_value: true,
        }),
      ),
    );
    render(<FastModeToggle pushToast={pushToast} />);
    const sw = await screen.findByRole("switch", { name: "Fast mode" });
    await waitFor(() => expect(sw).toBeDisabled());
    // design.md: a disabled control states its reason inline, not in a
    // tooltip.
    expect(
      screen.getByText(/CLAUDE_CODE_DISABLE_FAST_MODE/),
    ).toBeInTheDocument();
    await userEvent.click(sw);
    expect(setImpl.fn).not.toHaveBeenCalled();
  });

  it("toggles per-session opt-in independently", async () => {
    render(<FastModeToggle pushToast={pushToast} />);
    const sw = await screen.findByRole("switch", {
      name: "Require per-session opt-in",
    });
    await waitFor(() => expect(sw).toHaveAttribute("aria-checked", "false"));
    await userEvent.click(sw);
    expect(perSessionImpl.fn).toHaveBeenCalledWith(true);
    expect(setImpl.fn).not.toHaveBeenCalled();
  });

  it("keeps per-session opt-in usable while the env var locks fast mode", async () => {
    // The env var disables fast mode; it says nothing about whether the
    // preference should persist, so that switch stays live.
    stateImpl.fn = vi.fn(() =>
      Promise.resolve(mkState({ user_writable: false, env_disabled: true })),
    );
    render(<FastModeToggle pushToast={pushToast} />);
    const perSession = await screen.findByRole("switch", {
      name: "Require per-session opt-in",
    });
    await waitFor(() => expect(perSession).not.toBeDisabled());
  });
});
