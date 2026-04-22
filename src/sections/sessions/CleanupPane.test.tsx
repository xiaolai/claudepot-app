import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { PrunePlan } from "../../types";

const sessionPrunePlanSpy = vi.fn();
const sessionPruneStartSpy = vi.fn();

vi.mock("../../api", () => ({
  api: {
    sessionPrunePlan: (...a: unknown[]) => sessionPrunePlanSpy(...a),
    sessionPruneStart: (...a: unknown[]) => sessionPruneStartSpy(...a),
  },
}));

import { CleanupPane } from "./CleanupPane";

function mkPlan(entries: number): PrunePlan {
  return {
    entries: Array.from({ length: entries }, (_, i) => ({
      session_id: `s${i}`,
      file_path: `/tmp/s${i}.jsonl`,
      project_path: `/repo/p${i}`,
      size_bytes: (i + 1) * 1_000_000,
      last_ts_ms: null,
      has_error: false,
      is_sidechain: false,
    })),
    total_bytes: entries * 1_000_000,
  };
}

beforeEach(() => {
  sessionPrunePlanSpy.mockReset();
  sessionPruneStartSpy.mockReset();
});

describe("CleanupPane", () => {
  it("disables Preview until at least one filter is set", () => {
    render(<CleanupPane />);
    expect(screen.getByRole("button", { name: /Preview/i })).toBeDisabled();
  });

  it("Preview with older-than enables Prune button on returned plan", async () => {
    sessionPrunePlanSpy.mockResolvedValue(mkPlan(2));
    render(<CleanupPane />);
    await userEvent.type(
      screen.getByLabelText("Older than days"),
      "30",
    );
    await userEvent.click(screen.getByRole("button", { name: /Preview/i }));
    await waitFor(() => {
      expect(screen.getByTestId("prune-preview")).toBeInTheDocument();
    });
    expect(screen.getByRole("button", { name: /Prune → Trash/i })).not.toBeDisabled();
  });

  it("Prune → Trash calls sessionPruneStart with the same filter shape", async () => {
    sessionPrunePlanSpy.mockResolvedValue(mkPlan(1));
    sessionPruneStartSpy.mockResolvedValue("op-123");
    const onOpChange = vi.fn();
    render(<CleanupPane onOpChange={onOpChange} />);
    await userEvent.type(screen.getByLabelText("Older than days"), "7");
    await userEvent.click(screen.getByRole("button", { name: /Preview/i }));
    await waitFor(() => screen.getByTestId("prune-preview"));
    await userEvent.click(screen.getByRole("button", { name: /Prune → Trash/i }));
    await waitFor(() => {
      expect(sessionPruneStartSpy).toHaveBeenCalledWith({
        older_than_secs: 7 * 86400,
        larger_than_bytes: null,
        project: [],
        has_error: null,
        is_sidechain: null,
      });
    });
    expect(onOpChange).toHaveBeenCalledWith("op-123");
  });

  it("clears the prior plan when a filter input changes", async () => {
    sessionPrunePlanSpy.mockResolvedValue(mkPlan(2));
    render(<CleanupPane />);
    const olderInput = screen.getByLabelText("Older than days");
    await userEvent.type(olderInput, "7");
    await userEvent.click(screen.getByRole("button", { name: /Preview/i }));
    await waitFor(() => screen.getByTestId("prune-preview"));
    await userEvent.clear(olderInput);
    await userEvent.type(olderInput, "30");
    // Preview should be gone — filter changed, stale plan cleared.
    await waitFor(() => {
      expect(screen.queryByTestId("prune-preview")).toBeNull();
    });
  });
});
