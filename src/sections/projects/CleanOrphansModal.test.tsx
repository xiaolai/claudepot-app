import { describe, expect, it, vi, beforeEach } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// API spies per command.
const previewSpy = vi.fn();
const startSpy = vi.fn();
const statusSpy = vi.fn();

vi.mock("../../api", () => ({
  api: {
    projectCleanPreview: (...args: unknown[]) => previewSpy(...args),
    projectCleanStart: (...args: unknown[]) => startSpy(...args),
    projectCleanStatus: (...args: unknown[]) => statusSpy(...args),
  },
}));

// Fake Tauri event plumbing: useTauriEvent calls listen() from this
// module. We capture the handler so the test can synthesize events.
let capturedHandler: ((event: { payload: unknown }) => void) | null = null;
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((_channel: string, handler: (e: { payload: unknown }) => void) => {
    capturedHandler = handler;
    return Promise.resolve(() => {
      capturedHandler = null;
    });
  }),
}));

import { CleanOrphansModal } from "./CleanOrphansModal";
import type {
  CleanPreview,
  CleanResult,
  OperationProgressEvent,
  ProjectInfo,
  RunningOpInfo,
} from "../../types";

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
    protected_count: 0,
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
    protected_paths_skipped: 0,
    ...overrides,
  };
}

function emit(opId: string, payload: Partial<OperationProgressEvent>) {
  if (!capturedHandler) throw new Error("no event handler captured");
  const full: OperationProgressEvent = {
    op_id: opId,
    phase: payload.phase ?? "op",
    status: payload.status ?? "complete",
    done: payload.done,
    total: payload.total,
    detail: payload.detail,
  };
  capturedHandler({ payload: full });
}

beforeEach(() => {
  previewSpy.mockReset();
  startSpy.mockReset();
  statusSpy.mockReset();
  capturedHandler = null;
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
    expect(
      screen.getByRole("button", { name: /Remove 1 project$/ }),
    ).toBeEnabled();
  });

  it("disables Remove when preview shows only unreachable projects", async () => {
    previewSpy.mockResolvedValue(
      preview({ orphans: [], orphans_found: 0, unreachable_skipped: 2 }),
    );
    render(<CleanOrphansModal onClose={() => {}} onDone={() => {}} />);

    await waitFor(() => expect(previewSpy).toHaveBeenCalled());
    expect(
      await screen.findByText(/unreachable source paths/),
    ).toBeInTheDocument();
    const removeBtn = screen.getByRole("button", { name: /Remove$/ });
    expect(removeBtn).toBeDisabled();
  });

  it("flags empty orphans in the list with an 'empty' tag", async () => {
    previewSpy.mockResolvedValue(
      preview({
        orphans: [
          makeProject({
            original_path: "/tmp/abandoned",
            session_count: 0,
            is_empty: true,
          }),
        ],
        orphans_found: 1,
      }),
    );
    render(<CleanOrphansModal onClose={() => {}} onDone={() => {}} />);

    expect(await screen.findByText("/tmp/abandoned")).toBeInTheDocument();
    expect(screen.getByText("empty")).toBeInTheDocument();
  });

  it("kicks off a start request on confirm, then renders progress, then result", async () => {
    previewSpy.mockResolvedValue(
      preview({
        orphans: [makeProject()],
        orphans_found: 1,
        total_bytes: 1_024,
      }),
    );
    startSpy.mockResolvedValue("op-test-123");
    const finalResult = result({
      orphans_removed: 1,
      bytes_freed: 1_024,
      claude_json_entries_removed: 1,
      history_lines_removed: 3,
      snapshot_paths: [
        "/home/u/.claudepot/repair/snapshots/1-batch-clean-config.json",
      ],
    });
    const statusInfo: RunningOpInfo = {
      op_id: "op-test-123",
      kind: "clean_projects",
      old_path: "",
      new_path: "",
      current_phase: null,
      sub_progress: null,
      status: "complete",
      started_unix_secs: 0,
      last_error: null,
      move_result: null,
      clean_result: finalResult,
      failed_journal_id: null,
    };
    statusSpy.mockResolvedValue(statusInfo);

    const onDone = vi.fn();
    render(<CleanOrphansModal onClose={() => {}} onDone={onDone} />);
    await waitFor(() => expect(previewSpy).toHaveBeenCalled());

    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: /Remove 1 project/ }),
    );

    await waitFor(() => expect(startSpy).toHaveBeenCalledTimes(1));
    // Progress-phase label renders.
    expect(await screen.findByText(/Rewriting/)).toBeInTheDocument();

    // Mid-run sub_progress event → counter updates.
    act(() => {
      emit("op-test-123", {
        phase: "remove-dirs",
        status: "running",
        done: 0,
        total: 1,
      });
      emit("op-test-123", {
        phase: "remove-dirs",
        status: "running",
        done: 1,
        total: 1,
      });
    });
    expect(await screen.findByText(/1 of 1 projects/)).toBeInTheDocument();

    // Terminal event → fetches status → renders result.
    act(() => {
      emit("op-test-123", { phase: "op", status: "complete" });
    });
    await waitFor(() => expect(statusSpy).toHaveBeenCalledWith("op-test-123"));
    expect(
      await screen.findByText(/Removed/, { selector: ".clean-summary" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/1-batch-clean-config\.json/),
    ).toBeInTheDocument();
    expect(onDone).toHaveBeenCalledTimes(1);
    expect(onDone.mock.calls[0][0].orphans_removed).toBe(1);
  });

  it("surfaces start errors (e.g. journal gate) as an alert", async () => {
    previewSpy.mockResolvedValue(
      preview({ orphans: [makeProject()], orphans_found: 1 }),
    );
    startSpy.mockRejectedValue(
      "refusing to clean while 1 rename journal(s) are pending.",
    );
    render(<CleanOrphansModal onClose={() => {}} onDone={() => {}} />);

    await waitFor(() => expect(previewSpy).toHaveBeenCalled());
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: /Remove 1 project/ }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      /refusing to clean/,
    );
  });

  it("renders the protected-paths preserved line when the count is non-zero", async () => {
    // Preview shows 2 orphans, 1 protected. Confirm copy must mention
    // the carve-out so the user knows sibling state will be preserved
    // for protected entries before they press Confirm.
    previewSpy.mockResolvedValue(
      preview({
        orphans: [makeProject(), makeProject({ sanitized_name: "-other" })],
        orphans_found: 2,
        total_bytes: 2_048,
        protected_count: 1,
      }),
    );
    render(<CleanOrphansModal onClose={() => {}} onDone={() => {}} />);
    await waitFor(() => expect(previewSpy).toHaveBeenCalled());

    expect(
      await screen.findByText(
        /on your protected list/i,
      ),
    ).toBeInTheDocument();
  });

  it("renders the protected-skipped result line after a clean", async () => {
    // Drives the post-clean modal branch: result.protected_paths_skipped > 0
    // must produce a "Preserved sibling state for N protected paths" line.
    previewSpy.mockResolvedValue(
      preview({
        orphans: [makeProject()],
        orphans_found: 1,
        total_bytes: 512,
      }),
    );
    startSpy.mockResolvedValue("op-protected-1");
    const finalResult = result({
      orphans_removed: 1,
      bytes_freed: 512,
      protected_paths_skipped: 1,
    });
    statusSpy.mockResolvedValue({
      op_id: "op-protected-1",
      kind: "clean_projects",
      old_path: "",
      new_path: "",
      current_phase: null,
      sub_progress: null,
      status: "complete",
      started_unix_secs: 0,
      last_error: null,
      move_result: null,
      clean_result: finalResult,
      failed_journal_id: null,
    });

    render(<CleanOrphansModal onClose={() => {}} onDone={() => {}} />);
    await waitFor(() => expect(previewSpy).toHaveBeenCalled());

    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: /Remove 1 project/ }),
    );

    act(() => {
      emit("op-protected-1", { phase: "op", status: "complete" });
    });
    await waitFor(() => expect(statusSpy).toHaveBeenCalled());
    expect(
      await screen.findByText(/Preserved sibling state for 1 protected path/i),
    ).toBeInTheDocument();
  });

  it("blocks Escape close during the running phase", async () => {
    previewSpy.mockResolvedValue(
      preview({ orphans: [makeProject()], orphans_found: 1 }),
    );
    // start resolves but the terminal event never fires, keeping us
    // stuck in the running state for this assertion.
    startSpy.mockResolvedValue("op-stuck");

    const onClose = vi.fn();
    render(<CleanOrphansModal onClose={onClose} onDone={() => {}} />);
    await waitFor(() => expect(previewSpy).toHaveBeenCalled());

    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: /Remove 1 project/ }),
    );
    expect(await screen.findByText(/Rewriting|Removing/)).toBeInTheDocument();

    await user.keyboard("{Escape}");
    expect(onClose).not.toHaveBeenCalled();
  });
});
