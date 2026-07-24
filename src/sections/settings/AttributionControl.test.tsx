import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { AttributionState } from "../../api/attribution";

const attributionStateMock = vi.fn();
const attributionSetMock = vi.fn();

vi.mock("../../api", () => ({
  api: {
    attributionState: (...a: unknown[]) => attributionStateMock(...a),
    attributionSet: (...a: unknown[]) => attributionSetMock(...a),
  },
}));

import { AttributionControl } from "./AttributionControl";

function state(over: Partial<AttributionState> = {}): AttributionState {
  return {
    mode: "default",
    commit: null,
    pr: null,
    include_co_authored_by: null,
    ...over,
  };
}

describe("AttributionControl", () => {
  beforeEach(() => {
    attributionStateMock.mockReset();
    attributionSetMock.mockReset();
  });
  afterEach(() => vi.restoreAllMocks());

  const toast = () => vi.fn();

  it("renders three modes with Default pressed", async () => {
    attributionStateMock.mockResolvedValue(state());
    render(<AttributionControl pushToast={toast()} />);
    const def = await screen.findByRole("button", { name: "Default" });
    expect(def).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Off" })).toHaveAttribute("aria-pressed", "false");
    expect(screen.getByRole("button", { name: "Custom" })).toBeInTheDocument();
  });

  it("selecting Off applies immediately", async () => {
    attributionStateMock.mockResolvedValue(state());
    attributionSetMock.mockResolvedValue(state({ mode: "off", commit: "", pr: "", include_co_authored_by: false }));
    const push = vi.fn();
    render(<AttributionControl pushToast={push} />);
    const off = await screen.findByRole("button", { name: "Off" });
    await userEvent.setup().click(off);
    await waitFor(() =>
      expect(attributionSetMock).toHaveBeenCalledWith("off", undefined, undefined),
    );
    expect(push).toHaveBeenCalledWith("info", expect.stringMatching(/off/i));
  });

  it("Custom opens an editor and Save writes the typed strings", async () => {
    attributionStateMock.mockResolvedValue(state());
    attributionSetMock.mockResolvedValue(
      state({ mode: "custom", commit: "Co-Authored-By: Me <me@x>", pr: "Made with AI" }),
    );
    const push = vi.fn();
    const user = userEvent.setup();
    render(<AttributionControl pushToast={push} />);

    await user.click(await screen.findByRole("button", { name: "Custom" }));
    const commit = screen.getByPlaceholderText(/Co-Authored-By/i);
    const pr = screen.getByPlaceholderText(/Generated with AI/i);
    await user.type(commit, "Co-Authored-By: Me <me@x>");
    await user.type(pr, "Made with AI");

    await user.click(screen.getByRole("button", { name: /save custom attribution/i }));
    await waitFor(() =>
      expect(attributionSetMock).toHaveBeenCalledWith(
        "custom",
        "Co-Authored-By: Me <me@x>",
        "Made with AI",
      ),
    );
  });

  it("prefills the Custom editor from existing settings", async () => {
    attributionStateMock.mockResolvedValue(
      state({ mode: "custom", commit: "existing-commit", pr: "existing-pr" }),
    );
    render(<AttributionControl pushToast={toast()} />);
    // Custom already selected → editor visible with prefilled values.
    expect(await screen.findByDisplayValue("existing-commit")).toBeInTheDocument();
    expect(screen.getByDisplayValue("existing-pr")).toBeInTheDocument();
  });

  it("surfaces a load failure via pushToast", async () => {
    attributionStateMock.mockRejectedValue(new Error("boom"));
    const push = toast();
    render(<AttributionControl pushToast={push} />);
    await waitFor(() =>
      expect(push).toHaveBeenCalledWith("error", expect.stringContaining("boom")),
    );
  });
});
