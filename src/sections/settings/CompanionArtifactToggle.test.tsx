import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ArtifactState } from "../../api/artifact-tool";

const artifactToolStateMock = vi.fn();
const artifactToolSetMock = vi.fn();

vi.mock("../../api", () => ({
  api: {
    artifactToolState: (...a: unknown[]) => artifactToolStateMock(...a),
    artifactToolSet: (...a: unknown[]) => artifactToolSetMock(...a),
  },
}));

import { CompanionArtifactToggle } from "./CompanionArtifactToggle";

function state(over: Partial<ArtifactState> = {}): ArtifactState {
  return {
    enabled: true,
    decided_by: "default",
    user_writable: true,
    user_enable_value: null,
    user_disable_value: null,
    env_disable_set: false,
    ...over,
  };
}

describe("CompanionArtifactToggle", () => {
  beforeEach(() => {
    artifactToolStateMock.mockReset();
    artifactToolSetMock.mockReset();
  });
  afterEach(() => vi.restoreAllMocks());

  const toast = () => vi.fn();

  it("renders OFF by default (artifacts enabled → output not local)", async () => {
    artifactToolStateMock.mockResolvedValue(state());
    render(<CompanionArtifactToggle pushToast={toast()} />);
    const sw = await screen.findByRole("switch", {
      name: /keep companion output local/i,
    });
    expect(sw).toHaveAttribute("aria-checked", "false");
    expect(sw).not.toBeDisabled();
  });

  it("turning it on disables the Artifact tool (sets enabled=false)", async () => {
    artifactToolStateMock.mockResolvedValue(state());
    artifactToolSetMock.mockResolvedValue(
      state({ enabled: false, decided_by: "enable_setting", user_enable_value: false }),
    );
    const push = vi.fn();
    render(<CompanionArtifactToggle pushToast={push} />);
    const sw = await screen.findByRole("switch", {
      name: /keep companion output local/i,
    });
    await userEvent.setup().click(sw);
    // "keep local" ON must request the Artifact tool OFF.
    await waitFor(() => expect(artifactToolSetMock).toHaveBeenCalledWith(false));
    await waitFor(() => expect(sw).toHaveAttribute("aria-checked", "true"));
    // Success toast wording must match the direction — a swapped message fails here.
    expect(push).toHaveBeenCalledWith("info", expect.stringMatching(/kept local/i));
  });

  it("shows ON when output is already kept local, and turning off re-enables", async () => {
    artifactToolStateMock.mockResolvedValue(
      state({ enabled: false, decided_by: "enable_setting", user_enable_value: false }),
    );
    artifactToolSetMock.mockResolvedValue(state());
    const push = vi.fn();
    render(<CompanionArtifactToggle pushToast={push} />);
    const sw = await screen.findByRole("switch", {
      name: /keep companion output local/i,
    });
    expect(sw).toHaveAttribute("aria-checked", "true");
    await userEvent.setup().click(sw);
    // Turning "keep local" OFF must request the Artifact tool ON.
    await waitFor(() => expect(artifactToolSetMock).toHaveBeenCalledWith(true));
    expect(push).toHaveBeenCalledWith("info", expect.stringMatching(/re-enabled/i));
  });

  it("keeps the switch put and toasts when the write fails", async () => {
    artifactToolStateMock.mockResolvedValue(state());
    artifactToolSetMock.mockRejectedValue(new Error("boom"));
    const push = vi.fn();
    render(<CompanionArtifactToggle pushToast={push} />);
    const sw = await screen.findByRole("switch", {
      name: /keep companion output local/i,
    });
    expect(sw).toHaveAttribute("aria-checked", "false");
    await userEvent.setup().click(sw);
    await waitFor(() =>
      expect(push).toHaveBeenCalledWith("error", expect.stringContaining("boom")),
    );
    // A failed write must not flip the displayed state.
    expect(sw).toHaveAttribute("aria-checked", "false");
  });

  it("locks the toggle and states the reason when the env var overrides", async () => {
    artifactToolStateMock.mockResolvedValue(
      state({ enabled: false, decided_by: "env_disable", user_writable: false, env_disable_set: true }),
    );
    render(<CompanionArtifactToggle pushToast={toast()} />);
    const sw = await screen.findByRole("switch", {
      name: /keep companion output local/i,
    });
    expect(sw).toBeDisabled();
    // Inline reason names the overriding env var (design.md).
    expect(
      screen.getByText(/CLAUDE_CODE_DISABLE_ARTIFACT/),
    ).toBeInTheDocument();
    // A locked toggle never calls the setter.
    await userEvent.setup().click(sw);
    expect(artifactToolSetMock).not.toHaveBeenCalled();
  });

  it("surfaces a load failure via pushToast", async () => {
    artifactToolStateMock.mockRejectedValue(new Error("boom"));
    const push = toast();
    render(<CompanionArtifactToggle pushToast={push} />);
    await waitFor(() =>
      expect(push).toHaveBeenCalledWith("error", expect.stringContaining("boom")),
    );
  });
});
