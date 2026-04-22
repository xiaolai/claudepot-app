import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { BulkSlimPlan, PrunePlan } from "../../types";

const sessionPrunePlanSpy = vi.fn();
const sessionPruneStartSpy = vi.fn();
const sessionSlimPlanAllSpy = vi.fn();
const sessionSlimStartAllSpy = vi.fn();

vi.mock("../../api", () => ({
  api: {
    sessionPrunePlan: (...a: unknown[]) => sessionPrunePlanSpy(...a),
    sessionPruneStart: (...a: unknown[]) => sessionPruneStartSpy(...a),
    sessionSlimPlanAll: (...a: unknown[]) => sessionSlimPlanAllSpy(...a),
    sessionSlimStartAll: (...a: unknown[]) => sessionSlimStartAllSpy(...a),
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

function mkSlimPlan(entries: number): BulkSlimPlan {
  return {
    entries: Array.from({ length: entries }, (_, i) => ({
      session_id: `s${i}`,
      file_path: `/tmp/s${i}.jsonl`,
      project_path: `/repo/p${i}`,
      plan: {
        original_bytes: 10_000_000 + i * 1_000_000,
        projected_bytes: 4_000_000 + i * 500_000,
        redact_count: 0,
        image_redact_count: 20 + i,
        document_redact_count: 0,
        tools_affected: [],
        bytes_saved: 6_000_000 + i * 500_000,
      },
    })),
    total_bytes_saved: entries * 6_000_000,
    total_image_redacts: entries * 20,
    total_document_redacts: 0,
    total_tool_result_redacts: 0,
  };
}

beforeEach(() => {
  sessionPrunePlanSpy.mockReset();
  sessionPruneStartSpy.mockReset();
  sessionSlimPlanAllSpy.mockReset();
  sessionSlimStartAllSpy.mockReset();
});

describe("CleanupPane", () => {
  it("disables Preview until at least one filter is set", () => {
    render(<CleanupPane />);
    expect(screen.getByRole("button", { name: /^Preview$/i })).toBeDisabled();
  });

  it("Preview with older-than enables Prune button on returned plan", async () => {
    sessionPrunePlanSpy.mockResolvedValue(mkPlan(2));
    render(<CleanupPane />);
    await userEvent.type(
      screen.getByLabelText("Older than days"),
      "30",
    );
    await userEvent.click(screen.getByRole("button", { name: /^Preview$/i }));
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
    await userEvent.click(screen.getByRole("button", { name: /^Preview$/i }));
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
    await userEvent.click(screen.getByRole("button", { name: /^Preview$/i }));
    await waitFor(() => screen.getByTestId("prune-preview"));
    await userEvent.clear(olderInput);
    await userEvent.type(olderInput, "30");
    // Preview should be gone — filter changed, stale plan cleared.
    await waitFor(() => {
      expect(screen.queryByTestId("prune-preview")).toBeNull();
    });
  });

  it("shows the slim subsection with Strip-images and Strip-documents chips", () => {
    render(<CleanupPane />);
    expect(screen.getByTestId("slim-subsection")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: /Strip images/i })).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: /Strip documents/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Preview slim/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Slim → Trash/i })).toBeDisabled();
  });

  it("slim Preview + Slim → Trash dispatches sessionSlimStartAll with the filter and flags", async () => {
    sessionSlimPlanAllSpy.mockResolvedValue(mkSlimPlan(3));
    sessionSlimStartAllSpy.mockResolvedValue("op-slim-1");
    const onOpChange = vi.fn();
    render(<CleanupPane onOpChange={onOpChange} />);
    // Pick a filter AND the strip-images flag.
    await userEvent.type(screen.getByLabelText("Older than days"), "7");
    await userEvent.click(screen.getByRole("switch", { name: /Strip images/i }));
    // Preview slim.
    await userEvent.click(screen.getByRole("button", { name: /Preview slim/i }));
    await waitFor(() => screen.getByTestId("slim-preview"));
    // Execute.
    await userEvent.click(screen.getByRole("button", { name: /Slim → Trash/i }));
    await waitFor(() => {
      expect(sessionSlimStartAllSpy).toHaveBeenCalledWith(
        {
          older_than_secs: 7 * 86400,
          larger_than_bytes: null,
          project: [],
          has_error: null,
          is_sidechain: null,
        },
        expect.objectContaining({
          strip_images: true,
          strip_documents: false,
        }),
      );
    });
    expect(onOpChange).toHaveBeenCalledWith("op-slim-1");
  });

  it("Preview slim stays disabled until both a filter and a flag are set", async () => {
    render(<CleanupPane />);
    const previewSlim = screen.getByRole("button", { name: /Preview slim/i });
    expect(previewSlim).toBeDisabled();
    // Filter only — still disabled (no flag).
    await userEvent.type(screen.getByLabelText("Older than days"), "7");
    expect(previewSlim).toBeDisabled();
    // Flag only — still disabled (no filter) — reset filter first.
    await userEvent.clear(screen.getByLabelText("Older than days"));
    await userEvent.click(screen.getByRole("switch", { name: /Strip images/i }));
    expect(previewSlim).toBeDisabled();
    // Both — enabled.
    await userEvent.type(screen.getByLabelText("Older than days"), "7");
    expect(previewSlim).not.toBeDisabled();
  });
});
