import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const configEffectiveMcpSpy = vi.fn();
vi.mock("../../api", () => ({
  api: { configEffectiveMcp: (...a: unknown[]) => configEffectiveMcpSpy(...a) },
}));

import { EffectiveMcpRenderer } from "./EffectiveMcpRenderer";

describe("EffectiveMcpRenderer", () => {
  it("starts in Interactive mode and re-requests when mode changes", async () => {
    configEffectiveMcpSpy.mockResolvedValue({
      enterprise_lockout: false,
      servers: [],
    });
    render(<EffectiveMcpRenderer cwd="/" />);
    await waitFor(() => {
      expect(configEffectiveMcpSpy).toHaveBeenCalledWith("interactive", "/");
    });

    const niBtn = screen.getByRole("radio", { name: /non-interactive/i });
    await userEvent.click(niBtn);
    await waitFor(() => {
      expect(configEffectiveMcpSpy).toHaveBeenCalledWith(
        "non_interactive",
        "/",
      );
    });
  });

  it("renders enterprise lockout banner when flag set", async () => {
    configEffectiveMcpSpy.mockResolvedValue({
      enterprise_lockout: true,
      servers: [
        {
          name: "e",
          source_scope: "policy:managed_file_composite",
          contributors: ["policy:managed_file_composite"],
          approval: "rejected",
          approval_reason: null,
          blocked_by: "enterprise_lockout",
          masked: { command: "x" },
        },
      ],
    });
    render(<EffectiveMcpRenderer cwd="/" />);
    await waitFor(() =>
      expect(
        screen.getByText(/Enterprise policy in effect/i),
      ).toBeInTheDocument(),
    );
  });

  it("renders an approval badge per server row", async () => {
    configEffectiveMcpSpy.mockResolvedValue({
      enterprise_lockout: false,
      servers: [
        {
          name: "foo",
          source_scope: "project",
          contributors: ["project"],
          approval: "pending",
          approval_reason: null,
          blocked_by: null,
          masked: { command: "run-foo" },
        },
      ],
    });
    render(<EffectiveMcpRenderer cwd="/" />);
    await waitFor(() => {
      expect(screen.getByText("foo")).toBeInTheDocument();
    });
    expect(screen.getByText("pending")).toBeInTheDocument();
  });
});
