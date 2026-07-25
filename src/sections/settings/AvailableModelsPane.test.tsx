import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AvailableModelsState } from "../../api/availableModels";
import { AvailableModelsPane } from "./AvailableModelsPane";

const stateImpl = { fn: vi.fn() };
const setImpl = { fn: vi.fn() };

vi.mock("../../api", () => ({
  api: {
    availableModelsState: () => stateImpl.fn(),
    availableModelsSet: (entries: string[], enforce: boolean) =>
      setImpl.fn(entries, enforce),
  },
}));

function mkState(over: Partial<AvailableModelsState> = {}): AvailableModelsState {
  const entries = over.entries ?? [];
  const enforce = over.enforce ?? null;
  return {
    entries,
    enforce,
    key_present: entries.length > 0,
    restricts_models: entries.length > 0,
    enforce_is_effective: enforce === true && entries.length > 0,
    enforce_min_cc_version: "2.1.175",
    ...over,
  };
}

const pushToast = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  stateImpl.fn = vi.fn(() => Promise.resolve(mkState()));
  setImpl.fn = vi.fn((entries: string[], enforce: boolean) =>
    Promise.resolve(mkState({ entries, enforce })),
  );
});

describe("AvailableModelsPane", () => {
  it("says every model is selectable when the list is empty", async () => {
    render(<AvailableModelsPane pushToast={pushToast} />);
    expect(
      await screen.findByText(/every model your account can reach/i),
    ).toBeInTheDocument();
  });

  it("adds an entry and saves it", async () => {
    render(<AvailableModelsPane pushToast={pushToast} />);
    const input = await screen.findByLabelText("Model to allow");
    await userEvent.type(input, "sonnet");
    await userEvent.click(screen.getByRole("button", { name: /add/i }));
    expect(setImpl.fn).toHaveBeenCalledWith(["sonnet"], false);
    expect(await screen.findByText("sonnet")).toBeInTheDocument();
  });

  it("adds on Enter as well as the button", async () => {
    render(<AvailableModelsPane pushToast={pushToast} />);
    const input = await screen.findByLabelText("Model to allow");
    await userEvent.type(input, "opus{Enter}");
    expect(setImpl.fn).toHaveBeenCalledWith(["opus"], false);
  });

  it("renders entries in file order and lets them be reordered", async () => {
    // Order is load-bearing: with enforcement on, Default resolves to
    // the first entry, so the pane must never silently sort.
    stateImpl.fn = vi.fn(() =>
      Promise.resolve(mkState({ entries: ["sonnet", "opus"] })),
    );
    render(<AvailableModelsPane pushToast={pushToast} />);
    await screen.findByText("sonnet");
    await userEvent.click(
      screen.getByRole("button", { name: "Move opus up" }),
    );
    expect(setImpl.fn).toHaveBeenCalledWith(["opus", "sonnet"], false);
  });

  it("cannot move the first entry up or the last down", async () => {
    stateImpl.fn = vi.fn(() =>
      Promise.resolve(mkState({ entries: ["sonnet", "opus"] })),
    );
    render(<AvailableModelsPane pushToast={pushToast} />);
    await screen.findByText("sonnet");
    expect(screen.getByRole("button", { name: "Move sonnet up" })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Move opus down" }),
    ).toBeDisabled();
  });

  it("removes an entry", async () => {
    stateImpl.fn = vi.fn(() =>
      Promise.resolve(mkState({ entries: ["sonnet", "opus"] })),
    );
    render(<AvailableModelsPane pushToast={pushToast} />);
    await screen.findByText("sonnet");
    await userEvent.click(screen.getByRole("button", { name: "Remove sonnet" }));
    expect(setImpl.fn).toHaveBeenCalledWith(["opus"], false);
  });

  it("ignores a duplicate rather than saving it", async () => {
    stateImpl.fn = vi.fn(() => Promise.resolve(mkState({ entries: ["opus"] })));
    render(<AvailableModelsPane pushToast={pushToast} />);
    await screen.findByText("opus");
    const input = screen.getByLabelText("Model to allow");
    await userEvent.type(input, "opus{Enter}");
    expect(setImpl.fn).not.toHaveBeenCalled();
  });

  it("explains that enforcement is inert while the list is empty", async () => {
    // CC ignores enforceAvailableModels with an empty list, so claiming
    // Default is restricted would be a lie.
    stateImpl.fn = vi.fn(() =>
      Promise.resolve(
        mkState({ entries: [], enforce: true, enforce_is_effective: false }),
      ),
    );
    render(<AvailableModelsPane pushToast={pushToast} />);
    expect(await screen.findByText(/Set, but inert/)).toBeInTheDocument();
  });

  it("explains the Default-model hole when enforcement is off", async () => {
    stateImpl.fn = vi.fn(() =>
      Promise.resolve(mkState({ entries: ["opus"], enforce: false })),
    );
    render(<AvailableModelsPane pushToast={pushToast} />);
    expect(
      await screen.findByText(/bypasses the list entirely/),
    ).toBeInTheDocument();
  });

  it("saves the enforce flag alongside the current entries", async () => {
    stateImpl.fn = vi.fn(() => Promise.resolve(mkState({ entries: ["opus"] })));
    render(<AvailableModelsPane pushToast={pushToast} />);
    const sw = await screen.findByRole("switch", {
      name: "Apply the list to the Default model",
    });
    await userEvent.click(sw);
    expect(setImpl.fn).toHaveBeenCalledWith(["opus"], true);
  });

  it("shows the entries the backend normalized, not the draft", async () => {
    // The backend trims and dedupes; the editor must reflect the file.
    setImpl.fn = vi.fn(() => Promise.resolve(mkState({ entries: ["opus"] })));
    render(<AvailableModelsPane pushToast={pushToast} />);
    const input = await screen.findByLabelText("Model to allow");
    await userEvent.type(input, "  opus  {Enter}");
    await waitFor(() => expect(screen.getByText("opus")).toBeInTheDocument());
  });
});
