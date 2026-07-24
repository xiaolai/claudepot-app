import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ThinkingState } from "../../api/thinking";

const thinkingStateMock = vi.fn();
const thinkingSetMock = vi.fn();

vi.mock("../../api", () => ({
  api: {
    thinkingState: (...a: unknown[]) => thinkingStateMock(...a),
    thinkingSet: (...a: unknown[]) => thinkingSetMock(...a),
  },
}));

import { ExtendedThinkingToggle } from "./ExtendedThinkingToggle";

function state(over: Partial<ThinkingState> = {}): ThinkingState {
  return {
    effective: true,
    decided_by: "default",
    user_writable: true,
    user_settings_value: null,
    env_max_thinking_tokens_set: false,
    ...over,
  };
}

describe("ExtendedThinkingToggle", () => {
  beforeEach(() => {
    thinkingStateMock.mockReset();
    thinkingSetMock.mockReset();
  });
  afterEach(() => vi.restoreAllMocks());

  const toast = () => vi.fn();

  it("renders ON by default (thinking enabled)", async () => {
    thinkingStateMock.mockResolvedValue(state());
    render(<ExtendedThinkingToggle pushToast={toast()} />);
    const sw = await screen.findByRole("switch", {
      name: /extended thinking by default/i,
    });
    expect(sw).toHaveAttribute("aria-checked", "true");
    expect(sw).not.toBeDisabled();
  });

  it("turning it off writes alwaysThinkingEnabled=false", async () => {
    thinkingStateMock.mockResolvedValue(state());
    thinkingSetMock.mockResolvedValue(
      state({ effective: false, decided_by: "user_settings", user_settings_value: false }),
    );
    const push = vi.fn();
    render(<ExtendedThinkingToggle pushToast={push} />);
    const sw = await screen.findByRole("switch", {
      name: /extended thinking by default/i,
    });
    await userEvent.setup().click(sw);
    await waitFor(() => expect(thinkingSetMock).toHaveBeenCalledWith(false));
    await waitFor(() => expect(sw).toHaveAttribute("aria-checked", "false"));
    expect(push).toHaveBeenCalledWith("info", expect.stringMatching(/off by default/i));
  });

  it("locks the toggle and names MAX_THINKING_TOKENS when the env var overrides", async () => {
    thinkingStateMock.mockResolvedValue(
      state({ decided_by: "env_max_thinking_tokens", user_writable: false, env_max_thinking_tokens_set: true }),
    );
    render(<ExtendedThinkingToggle pushToast={toast()} />);
    const sw = await screen.findByRole("switch", {
      name: /extended thinking by default/i,
    });
    expect(sw).toBeDisabled();
    expect(screen.getByText(/MAX_THINKING_TOKENS/)).toBeInTheDocument();
    await userEvent.setup().click(sw);
    expect(thinkingSetMock).not.toHaveBeenCalled();
  });

  it("keeps the switch put and toasts when the write fails", async () => {
    thinkingStateMock.mockResolvedValue(state());
    thinkingSetMock.mockRejectedValue(new Error("boom"));
    const push = vi.fn();
    render(<ExtendedThinkingToggle pushToast={push} />);
    const sw = await screen.findByRole("switch", {
      name: /extended thinking by default/i,
    });
    await userEvent.setup().click(sw);
    await waitFor(() =>
      expect(push).toHaveBeenCalledWith("error", expect.stringContaining("boom")),
    );
    expect(sw).toHaveAttribute("aria-checked", "true");
  });
});
