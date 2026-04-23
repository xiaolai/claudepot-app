import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

const configEffectiveSettingsSpy = vi.fn();
vi.mock("../../api", () => ({
  api: { configEffectiveSettings: (...a: unknown[]) => configEffectiveSettingsSpy(...a) },
}));

import { EffectiveRenderer } from "./EffectiveRenderer";

describe("EffectiveRenderer", () => {
  it("renders a policy banner when policy_winner is set", async () => {
    configEffectiveSettingsSpy.mockResolvedValue({
      merged: { theme: "dark" },
      provenance: [
        {
          path: "theme",
          winner: "policy:managed_file_composite",
          contributors: ["policy:managed_file_composite"],
          suppressed: false,
        },
      ],
      policy_winner: "managed_file_composite",
      policy_errors: [],
    });
    render(<EffectiveRenderer cwd="/" />);
    await waitFor(() =>
      expect(
        screen.getByText(/Policy active:/i),
      ).toBeInTheDocument(),
    );
    expect(screen.getByText("managed_file_composite")).toBeInTheDocument();
  });

  it("badges each primitive leaf with its winning scope", async () => {
    configEffectiveSettingsSpy.mockResolvedValue({
      merged: { theme: "dark", verbose: true },
      provenance: [
        {
          path: "theme",
          winner: "user",
          contributors: ["user"],
          suppressed: false,
        },
        {
          path: "verbose",
          winner: "project",
          contributors: ["project", "user"],
          suppressed: false,
        },
      ],
      policy_winner: null,
      policy_errors: [],
    });
    render(<EffectiveRenderer cwd="/" />);
    await waitFor(() =>
      expect(screen.getByText('"dark"')).toBeInTheDocument(),
    );
    expect(screen.getByText("user")).toBeInTheDocument();
    expect(screen.getByText("project")).toBeInTheDocument();
  });
});
