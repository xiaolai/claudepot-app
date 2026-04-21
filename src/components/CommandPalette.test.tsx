import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

vi.mock("../api", () => ({
  api: {
    sessionSearch: vi.fn().mockResolvedValue([]),
  },
}));

import { CommandPalette } from "./CommandPalette";
import { sampleStatus } from "../test/fixtures";

describe("CommandPalette — empty-state keyboard behavior", () => {
  const baseProps = {
    accounts: [],
    status: sampleStatus(),
    onClose: vi.fn(),
    onSwitchCli: vi.fn(),
    onSwitchDesktop: vi.fn(),
    onAdd: vi.fn(),
    onRefresh: vi.fn(),
    onRemove: vi.fn(),
    onNavigate: vi.fn(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("ArrowDown is a no-op when there are zero results (no crash, no dispatch)", async () => {
    const user = userEvent.setup();
    render(<CommandPalette {...baseProps} />);

    const input = screen.getByPlaceholderText(/Search accounts, actions/i);
    // Query that matches nothing. Short enough that useSessionSearch
    // (>=2 chars required) still skips the network call, so no hits arrive.
    await user.type(input, "zzzzzzzzzzzzzqqq");

    // Pressing ArrowDown on a zero-result list must not throw and must
    // not open the goto-session dispatch on Enter.
    fireEvent.keyDown(input, { key: "ArrowDown" });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(baseProps.onSwitchCli).not.toHaveBeenCalled();
    expect(baseProps.onAdd).not.toHaveBeenCalled();
    expect(baseProps.onNavigate).not.toHaveBeenCalled();
    expect(baseProps.onClose).not.toHaveBeenCalled();
  });

  it("Escape triggers onClose", () => {
    render(<CommandPalette {...baseProps} />);

    const input = screen.getByPlaceholderText(/Search accounts, actions/i);
    fireEvent.keyDown(input, { key: "Escape" });

    expect(baseProps.onClose).toHaveBeenCalled();
  });

  it("does NOT render the 'Sessions' group header when filtered hits exist but zero session hits", async () => {
    const user = userEvent.setup();
    render(<CommandPalette {...baseProps} />);

    const input = screen.getByPlaceholderText(/Search accounts, actions/i);
    // 'add' fuzzy-matches the "Add account" palette action → filtered.length > 0
    // while sessionHits stays empty (mocked) and not loading.
    await user.type(input, "add");

    // Wait for useSessionSearch's loading state to settle (mock resolves to []).
    await waitFor(() => {
      expect(screen.queryByText(/…searching/)).not.toBeInTheDocument();
    });

    expect(screen.queryByText(/^Sessions/)).not.toBeInTheDocument();
  });
});
