import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Mock the api surface this pane uses. Each test resets the spies and
// configures per-call resolved values.
const listSpy = vi.fn();
const addSpy = vi.fn();
const removeSpy = vi.fn();
const resetSpy = vi.fn();
vi.mock("../../api", () => ({
  api: {
    protectedPathsList: (...a: unknown[]) => listSpy(...a),
    protectedPathsAdd: (...a: unknown[]) => addSpy(...a),
    protectedPathsRemove: (...a: unknown[]) => removeSpy(...a),
    protectedPathsReset: (...a: unknown[]) => resetSpy(...a),
  },
}));

import { ProtectedPathsPane } from "./ProtectedPathsPane";
import type { ProtectedPath } from "../../types";

const pushToast = vi.fn();

function row(path: string, source: "default" | "user"): ProtectedPath {
  return { path, source };
}

beforeEach(() => {
  listSpy.mockReset();
  addSpy.mockReset();
  removeSpy.mockReset();
  resetSpy.mockReset();
  pushToast.mockReset();
});

describe("ProtectedPathsPane", () => {
  it("renders the list returned by the api", async () => {
    listSpy.mockResolvedValue([row("/", "default"), row("/Volumes/x", "user")]);
    render(<ProtectedPathsPane pushToast={pushToast} />);
    await waitFor(() => expect(screen.getByText("/")).toBeInTheDocument());
    expect(screen.getByText("/Volumes/x")).toBeInTheDocument();
    // Source badges by text.
    expect(screen.getByText("default")).toBeInTheDocument();
    expect(screen.getByText("user")).toBeInTheDocument();
  });

  it("adds a path through the input, then refreshes", async () => {
    listSpy.mockResolvedValueOnce([]).mockResolvedValueOnce([row("/x", "user")]);
    addSpy.mockResolvedValue(row("/x", "user"));
    const user = userEvent.setup();
    render(<ProtectedPathsPane pushToast={pushToast} />);
    await waitFor(() => expect(listSpy).toHaveBeenCalledTimes(1));

    const input = screen.getByPlaceholderText(/path\/to\/protect/i);
    await user.type(input, "/x");
    await user.click(screen.getByRole("button", { name: /^Add$/i }));

    await waitFor(() => expect(addSpy).toHaveBeenCalledWith("/x"));
    await waitFor(() => expect(screen.getByText("/x")).toBeInTheDocument());
  });

  it("surfaces an inline error from the api when add fails", async () => {
    listSpy.mockResolvedValue([]);
    addSpy.mockRejectedValue("path is already protected: '/'");
    const user = userEvent.setup();
    render(<ProtectedPathsPane pushToast={pushToast} />);
    await waitFor(() => expect(listSpy).toHaveBeenCalled());

    const input = screen.getByPlaceholderText(/path\/to\/protect/i);
    await user.type(input, "/");
    await user.click(screen.getByRole("button", { name: /^Add$/i }));

    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent(/already protected/i),
    );
    // Inline error, not a toast.
    expect(pushToast).not.toHaveBeenCalled();
  });

  it("removes a row when the trailing button is clicked", async () => {
    listSpy
      .mockResolvedValueOnce([row("/tmp", "default")])
      .mockResolvedValueOnce([]);
    removeSpy.mockResolvedValue(undefined);
    const user = userEvent.setup();
    render(<ProtectedPathsPane pushToast={pushToast} />);
    await waitFor(() => expect(screen.getByText("/tmp")).toBeInTheDocument());

    await user.click(
      screen.getByRole("button", { name: /Remove \/tmp/i }),
    );
    await waitFor(() => expect(removeSpy).toHaveBeenCalledWith("/tmp"));
    await waitFor(() => expect(screen.queryByText("/tmp")).not.toBeInTheDocument());
  });

  it("resets to defaults and toasts confirmation", async () => {
    listSpy.mockResolvedValue([row("/x", "user")]);
    resetSpy.mockResolvedValue([row("/", "default")]);
    const user = userEvent.setup();
    render(<ProtectedPathsPane pushToast={pushToast} />);
    await waitFor(() => expect(screen.getByText("/x")).toBeInTheDocument());

    await user.click(screen.getByRole("button", { name: /reset to defaults/i }));
    await waitFor(() => expect(resetSpy).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByText("/")).toBeInTheDocument());
    expect(pushToast).toHaveBeenCalledWith("info", expect.stringMatching(/reset/i));
  });

  it("disables Add when input is whitespace-only", async () => {
    listSpy.mockResolvedValue([]);
    const user = userEvent.setup();
    render(<ProtectedPathsPane pushToast={pushToast} />);
    await waitFor(() => expect(listSpy).toHaveBeenCalled());

    const addBtn = screen.getByRole("button", { name: /^Add$/i });
    expect(addBtn).toBeDisabled();
    await user.type(screen.getByPlaceholderText(/path\/to\/protect/i), "   ");
    expect(addBtn).toBeDisabled();
  });
});
