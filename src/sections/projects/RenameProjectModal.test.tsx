import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, act, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Mock the Tauri plugin-dialog import so clicking "Browse…" doesn't
// explode in jsdom. Default: user cancels.
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(() => Promise.resolve(null)),
}));

// Mock the api module; each test overrides api.projectMoveDryRun as needed.
const dryRunSpy = vi.fn();
vi.mock("../../api", () => ({
  api: {
    projectMoveDryRun: (...args: unknown[]) => dryRunSpy(...args),
  },
}));

import { RenameProjectModal } from "./RenameProjectModal";
import type { DryRunPlan, MoveArgs } from "../../types";

function okPlan(overrides: Partial<DryRunPlan> = {}): DryRunPlan {
  return {
    would_move_dir: true,
    old_cc_dir: "/old/cc/dir",
    new_cc_dir: "/new/cc/dir",
    session_count: 3,
    cc_dir_size: 12345,
    estimated_history_lines: 10,
    conflict: null,
    estimated_jsonl_files: 5,
    would_rewrite_claude_json: true,
    would_move_memory_dir: false,
    would_rewrite_project_settings: false,
    ...overrides,
  };
}

beforeEach(() => {
  dryRunSpy.mockReset();
  vi.useRealTimers();
});

describe("RenameProjectModal", () => {
  it("renders the current path and opens focused on new path input", () => {
    dryRunSpy.mockResolvedValue(okPlan());
    render(
      <RenameProjectModal
        oldPath="/tmp/foo"
        onClose={() => {}}
        onSubmit={() => {}}
      />,
    );
    expect(screen.getByText("/tmp/foo")).toBeInTheDocument();
    const input = screen.getByLabelText(/New path/) as HTMLInputElement;
    expect(input.value).toBe("/tmp/foo");
    // autoFocus should put focus on the input.
    expect(document.activeElement).toBe(input);
  });

  it("debounces dry-run calls and renders preview", async () => {
    vi.useFakeTimers();
    dryRunSpy.mockResolvedValue(okPlan({ session_count: 7 }));

    render(
      <RenameProjectModal
        oldPath="/tmp/foo"
        onClose={() => {}}
        onSubmit={() => {}}
      />,
    );

    const input = screen.getByLabelText(/New path/) as HTMLInputElement;
    // Change input — setTimeout(300) should be pending.
    await act(async () => {
      input.focus();
      input.value = "/tmp/bar";
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(dryRunSpy).not.toHaveBeenCalled();

    await act(async () => {
      vi.advanceTimersByTime(DEBOUNCE_MS + 10);
    });
    expect(dryRunSpy).toHaveBeenCalled();

    vi.useRealTimers();
    await screen.findByText(/7 sessions/);
  });

  it("disables Rename button when new path equals old", async () => {
    dryRunSpy.mockResolvedValue(okPlan());
    render(
      <RenameProjectModal
        oldPath="/tmp/foo"
        onClose={() => {}}
        onSubmit={() => {}}
      />,
    );
    // Initial: newPath = oldPath → disabled.
    const btn = screen.getByRole("button", { name: "Rename" });
    expect(btn).toBeDisabled();
  });

  it("disables Rename when preview reports a conflict and collision=none", async () => {
    dryRunSpy.mockResolvedValue(okPlan({ conflict: "target exists" }));
    const user = userEvent.setup();
    render(
      <RenameProjectModal
        oldPath="/tmp/foo"
        onClose={() => {}}
        onSubmit={() => {}}
      />,
    );

    const input = screen.getByLabelText(/New path/);
    await user.clear(input);
    await user.type(input, "/tmp/bar");

    await waitFor(() => {
      expect(screen.getByText(/Conflict:/)).toBeInTheDocument();
    });

    const btn = screen.getByRole("button", { name: "Rename" });
    expect(btn).toBeDisabled();

    // Selecting Merge resolves the gate.
    await user.click(screen.getByLabelText(/Merge \(old wins\)/));
    await waitFor(() => {
      expect(btn).toBeEnabled();
    });
  });

  it("passes merge/overwrite flags through on submit", async () => {
    dryRunSpy.mockResolvedValue(okPlan());
    const user = userEvent.setup();
    let submitted: MoveArgs | null = null;
    render(
      <RenameProjectModal
        oldPath="/tmp/foo"
        onClose={() => {}}
        onSubmit={(args) => {
          submitted = args;
        }}
      />,
    );

    const input = screen.getByLabelText(/New path/);
    await user.clear(input);
    await user.type(input, "/tmp/bar");
    await user.click(screen.getByLabelText(/Overwrite/));
    await user.click(screen.getByLabelText(/--force/));

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Rename" })).toBeEnabled();
    });
    await user.click(screen.getByRole("button", { name: "Rename" }));

    expect(submitted).not.toBeNull();
    expect(submitted!.overwrite).toBe(true);
    expect(submitted!.merge).toBe(false);
    expect(submitted!.force).toBe(true);
    expect(submitted!.oldPath).toBe("/tmp/foo");
    expect(submitted!.newPath).toBe("/tmp/bar");
  });

  it("Escape fires onClose", async () => {
    dryRunSpy.mockResolvedValue(okPlan());
    const close = vi.fn();
    render(
      <RenameProjectModal
        oldPath="/tmp/foo"
        onClose={close}
        onSubmit={() => {}}
      />,
    );
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(close).toHaveBeenCalled();
  });

  it("shows 'Preview is approximate' disclaimer near the Rename button", () => {
    dryRunSpy.mockResolvedValue(okPlan());
    render(
      <RenameProjectModal
        oldPath="/tmp/foo"
        onClose={() => {}}
        onSubmit={() => {}}
      />,
    );
    expect(
      screen.getByText(/Preview is approximate/),
    ).toBeInTheDocument();
  });
});

const DEBOUNCE_MS = 300;
