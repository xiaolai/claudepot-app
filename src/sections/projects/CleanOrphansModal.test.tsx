import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Spies assigned per test; the mock resolves to whatever the spy returns.
const previewSpy = vi.fn();
const executeSpy = vi.fn();

vi.mock("../../api", () => ({
  api: {
    projectCleanPreview: (...args: unknown[]) => previewSpy(...args),
    projectCleanExecute: (...args: unknown[]) => executeSpy(...args),
  },
}));

import { CleanOrphansModal } from "./CleanOrphansModal";
import type { CleanPreview, CleanResult, ProjectInfo } from "../../types";

function makeProject(overrides: Partial<ProjectInfo> = {}): ProjectInfo {
  return {
    sanitized_name: "-tmp-gone",
    original_path: "/tmp/gone",
    session_count: 2,
    memory_file_count: 0,
    total_size_bytes: 1_024,
    last_modified_ms: null,
    is_orphan: true,
    is_reachable: true,
    is_empty: false,
    ...overrides,
  };
}

function preview(overrides: Partial<CleanPreview> = {}): CleanPreview {
  return {
    orphans: [],
    orphans_found: 0,
    unreachable_skipped: 0,
    total_bytes: 0,
    ...overrides,
  };
}

function result(overrides: Partial<CleanResult> = {}): CleanResult {
  return {
    orphans_found: 0,
    orphans_removed: 0,
    orphans_skipped_live: 0,
    unreachable_skipped: 0,
    bytes_freed: 0,
    claude_json_entries_removed: 0,
    history_lines_removed: 0,
    claudepot_artifacts_removed: 0,
    snapshot_paths: [],
    ...overrides,
  };
}

beforeEach(() => {
  previewSpy.mockReset();
  executeSpy.mockReset();
});

describe("CleanOrphansModal", () => {
  it("loads preview on mount and renders the orphan list", async () => {
    previewSpy.mockResolvedValue(
      preview({
        orphans: [makeProject({ original_path: "/tmp/gone-a" })],
        orphans_found: 1,
        total_bytes: 1_024,
      }),
    );
    render(<CleanOrphansModal onClose={() => {}} onDone={() => {}} />);

    await waitFor(() => expect(previewSpy).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("/tmp/gone-a")).toBeInTheDocument();
    // Confirm button shows the count.
    expect(
      screen.getByRole("button", { name: /Remove 1 project$/ }),
    ).toBeEnabled();
  });

  it("disables confirm when there are no orphans (only unreachable)", async () => {
    previewSpy.mockResolvedValue(
      preview({ orphans: [], orphans_found: 0, unreachable_skipped: 2 }),
    );
    render(<CleanOrphansModal onClose={() => {}} onDone={() => {}} />);

    await waitFor(() => expect(previewSpy).toHaveBeenCalled());
    // The unreachable notice is visible.
    expect(
      await screen.findByText(/unreachable source paths/),
    ).toBeInTheDocument();
    // The Remove button is present (no orphan count) but disabled.
    const removeBtn = screen.getByRole("button", { name: /Remove$/ });
    expect(removeBtn).toBeDisabled();
  });

  it("shows an 'empty' tag on empty orphans in the preview list", async () => {
    previewSpy.mockResolvedValue(
      preview({
        orphans: [
          makeProject({
            original_path: "/tmp/abandoned",
            session_count: 0,
            is_empty: true,
            is_orphan: true,
          }),
        ],
        orphans_found: 1,
      }),
    );
    render(<CleanOrphansModal onClose={() => {}} onDone={() => {}} />);

    expect(await screen.findByText("/tmp/abandoned")).toBeInTheDocument();
    expect(screen.getByText("empty")).toBeInTheDocument();
  });

  it("runs clean on confirm and renders the result with snapshot paths", async () => {
    previewSpy.mockResolvedValue(
      preview({
        orphans: [makeProject()],
        orphans_found: 1,
        total_bytes: 1_024,
      }),
    );
    executeSpy.mockResolvedValue(
      result({
        orphans_found: 1,
        orphans_removed: 1,
        bytes_freed: 1_024,
        claude_json_entries_removed: 1,
        history_lines_removed: 3,
        snapshot_paths: [
          "/home/u/.claude/claudepot/snapshots/1-abc-clean-config.json",
          "/home/u/.claude/claudepot/snapshots/1-abc-clean-history.json",
        ],
      }),
    );
    const onDone = vi.fn();
    render(<CleanOrphansModal onClose={() => {}} onDone={onDone} />);

    await waitFor(() => expect(previewSpy).toHaveBeenCalled());
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: /Remove 1 project/ }),
    );

    // Result panel appears.
    expect(
      await screen.findByText(/Removed/, { selector: ".clean-summary" }),
    ).toBeInTheDocument();
    // Snapshot paths are listed.
    expect(
      screen.getByText(/1-abc-clean-config\.json/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/1-abc-clean-history\.json/),
    ).toBeInTheDocument();

    // onDone callback fires with the result for the parent's toast.
    expect(onDone).toHaveBeenCalledTimes(1);
    expect(onDone.mock.calls[0][0].orphans_removed).toBe(1);
  });

  it("surfaces backend errors (e.g. journal gate) in an alert", async () => {
    previewSpy.mockResolvedValue(preview({ orphans_found: 0 }));
    executeSpy.mockRejectedValue(
      "refusing to clean while 1 rename journal(s) are pending.",
    );
    render(<CleanOrphansModal onClose={() => {}} onDone={() => {}} />);

    await waitFor(() => expect(previewSpy).toHaveBeenCalled());
    // No orphans: Remove is disabled; directly trigger execute via a seeded
    // scenario. Simulate by starting from a preview that has orphans.
    previewSpy.mockResolvedValueOnce(
      preview({ orphans: [makeProject()], orphans_found: 1 }),
    );
    // Re-render with orphans so we can click Remove.
    const { unmount } = render(
      <CleanOrphansModal onClose={() => {}} onDone={() => {}} />,
    );
    const user = userEvent.setup();
    const btn = await screen.findByRole("button", { name: /Remove 1 project/ });
    await user.click(btn);

    expect(
      await screen.findByRole("alert"),
    ).toHaveTextContent(/refusing to clean/);
    unmount();
  });

  it("does NOT close on Escape while the clean is running", async () => {
    previewSpy.mockResolvedValue(
      preview({ orphans: [makeProject()], orphans_found: 1 }),
    );
    // Make execute hang so we can inspect the running state.
    let releaseExecute: (value: CleanResult) => void = () => {};
    executeSpy.mockImplementation(
      () =>
        new Promise<CleanResult>((resolve) => {
          releaseExecute = resolve;
        }),
    );
    const onClose = vi.fn();
    render(<CleanOrphansModal onClose={onClose} onDone={() => {}} />);
    await waitFor(() => expect(previewSpy).toHaveBeenCalled());

    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: /Remove 1 project/ }),
    );

    // Now in running state.
    expect(await screen.findByText(/Cleaning…/)).toBeInTheDocument();

    // Escape must be a no-op.
    await user.keyboard("{Escape}");
    expect(onClose).not.toHaveBeenCalled();

    // Let the promise resolve to avoid a dangling timer.
    releaseExecute(result());
  });
});
