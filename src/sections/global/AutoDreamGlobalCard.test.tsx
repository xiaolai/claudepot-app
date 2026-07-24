import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { AutoDreamState } from "../../api/auto-dream";

const autoDreamStateMock = vi.fn();
const autoDreamSetMock = vi.fn();
const pushToastMock = vi.fn();

vi.mock("../../api", () => ({
  api: {
    autoDreamState: (...a: unknown[]) => autoDreamStateMock(...a),
    autoDreamSet: (...a: unknown[]) => autoDreamSetMock(...a),
  },
}));

vi.mock("../../providers/AppStateProvider", () => ({
  useAppState: () => ({ pushToast: pushToastMock }),
}));

import { AutoDreamGlobalCard } from "./AutoDreamGlobalCard";

function state(over: Partial<AutoDreamState> = {}): AutoDreamState {
  return {
    mode: "default",
    user_settings_value: null,
    auto_memory_enabled: true,
    ...over,
  };
}

describe("AutoDreamGlobalCard", () => {
  beforeEach(() => {
    autoDreamStateMock.mockReset();
    autoDreamSetMock.mockReset();
    pushToastMock.mockReset();
  });
  afterEach(() => vi.restoreAllMocks());

  it("renders three modes with Default pressed", async () => {
    autoDreamStateMock.mockResolvedValue(state());
    render(<AutoDreamGlobalCard />);
    const def = await screen.findByRole("button", { name: "Default" });
    expect(def).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "On" })).not.toBeDisabled();
  });

  it("selecting On writes mode=on", async () => {
    autoDreamStateMock.mockResolvedValue(state());
    autoDreamSetMock.mockResolvedValue(state({ mode: "on", user_settings_value: true }));
    render(<AutoDreamGlobalCard />);
    const on = await screen.findByRole("button", { name: "On" });
    await userEvent.setup().click(on);
    await waitFor(() => expect(autoDreamSetMock).toHaveBeenCalledWith("on"));
    expect(pushToastMock).toHaveBeenCalledWith("info", expect.stringMatching(/on/i));
  });

  it("disables the control and states the dependency when auto-memory is off", async () => {
    autoDreamStateMock.mockResolvedValue(state({ auto_memory_enabled: false }));
    render(<AutoDreamGlobalCard />);
    const on = await screen.findByRole("button", { name: "On" });
    expect(on).toBeDisabled();
    expect(screen.getByText(/requires auto-memory/i)).toBeInTheDocument();
    await userEvent.setup().click(on);
    expect(autoDreamSetMock).not.toHaveBeenCalled();
  });

  it("surfaces a load failure via pushToast", async () => {
    autoDreamStateMock.mockRejectedValue(new Error("boom"));
    render(<AutoDreamGlobalCard />);
    await waitFor(() =>
      expect(pushToastMock).toHaveBeenCalledWith("error", expect.stringContaining("boom")),
    );
  });
});
