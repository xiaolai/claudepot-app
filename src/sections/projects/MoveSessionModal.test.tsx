import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { MoveSessionReport, ProjectInfo } from "../../types";

const moveSpy = vi.fn();
vi.mock("../../api", () => ({
  api: {
    sessionMove: (...args: unknown[]) => moveSpy(...args),
  },
}));
const openDialogSpy = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: unknown[]) => openDialogSpy(...args),
}));

import { MoveSessionModal } from "./MoveSessionModal";

function mkProject(overrides: Partial<ProjectInfo> = {}): ProjectInfo {
  return {
    sanitized_name: "-live",
    original_path: "/live",
    session_count: 0,
    memory_file_count: 0,
    total_size_bytes: 0,
    last_modified_ms: null,
    is_orphan: false,
    is_reachable: true,
    is_empty: false,
    ...overrides,
  };
}

function mkReport(overrides: Partial<MoveSessionReport> = {}): MoveSessionReport {
  return {
    sessionId: "abcd0000-0000-0000-0000-000000000000",
    fromSlug: "-from",
    toSlug: "-to",
    jsonlLinesRewritten: 12,
    subagentFilesMoved: 0,
    remoteAgentFilesMoved: 0,
    historyEntriesMoved: 3,
    historyEntriesUnmapped: 1,
    claudeJsonPointersCleared: 1,
    sourceDirRemoved: false,
    ...overrides,
  };
}

const baseProps = {
  sessionId: "abcd0000-0000-0000-0000-000000000000",
  fromCwd: "/from",
  projects: [
    mkProject({ original_path: "/from", sanitized_name: "-from" }),
    mkProject({ original_path: "/live/main", sanitized_name: "-live-main" }),
    mkProject({ original_path: "/live/other", sanitized_name: "-live-other" }),
  ],
};

describe("MoveSessionModal", () => {
  beforeEach(() => {
    moveSpy.mockReset();
    openDialogSpy.mockReset();
  });

  it("excludes the source cwd from the target picker", () => {
    render(
      <MoveSessionModal
        {...baseProps}
        onClose={() => {}}
        onCompleted={() => {}}
      />,
    );
    const select = screen.getByRole("combobox") as HTMLSelectElement;
    const values = Array.from(select.options).map((o) => o.value);
    expect(values).not.toContain("/from");
    expect(values).toContain("/live/main");
    expect(values).toContain("__other__");
  });

  it("calls the api with the selected target and reports success inline", async () => {
    moveSpy.mockResolvedValue(mkReport());
    const onCompleted = vi.fn();
    const user = userEvent.setup();

    render(
      <MoveSessionModal
        {...baseProps}
        onClose={() => {}}
        onCompleted={onCompleted}
      />,
    );
    await user.selectOptions(screen.getByRole("combobox"), "/live/other");
    await user.click(screen.getByRole("button", { name: /Move to/i }));

    await waitFor(() =>
      expect(moveSpy).toHaveBeenCalledWith({
        sessionId: baseProps.sessionId,
        fromCwd: "/from",
        toCwd: "/live/other",
        forceLive: false,
        forceConflict: false,
      }),
    );
    await waitFor(() => expect(screen.getByText(/^Moved\.$/)).toBeInTheDocument());
    expect(screen.getByText(/12/)).toBeInTheDocument(); // lines rewritten
    // The "1 stayed (pre-sessionId)" inline meta appears after success;
    // it's distinct from the same phrase in the preamble.
    expect(screen.getByText(/stayed \(pre-sessionId\)/i)).toBeInTheDocument();
    expect(onCompleted).toHaveBeenCalledTimes(1);
  });

  it("shows inline error on failure, no double-signal", async () => {
    moveSpy.mockRejectedValue("session appears live (mtime < threshold)");
    const user = userEvent.setup();
    render(
      <MoveSessionModal
        {...baseProps}
        onClose={() => {}}
        onCompleted={() => {}}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Move to/i }));

    await waitFor(() =>
      expect(screen.getByText(/appears live/)).toBeInTheDocument(),
    );
  });

  it('Other… reveals an input and "Browse" picks a path', async () => {
    openDialogSpy.mockResolvedValue("/picked");
    const user = userEvent.setup();
    render(
      <MoveSessionModal
        {...baseProps}
        onClose={() => {}}
        onCompleted={() => {}}
      />,
    );
    await user.selectOptions(screen.getByRole("combobox"), "__other__");
    expect(screen.getByPlaceholderText(/target cwd/i)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Browse/i }));
    await waitFor(() =>
      expect(screen.getByDisplayValue("/picked")).toBeInTheDocument(),
    );
  });

  it("threads forceLive / forceConflict into the api call", async () => {
    moveSpy.mockResolvedValue(mkReport());
    const user = userEvent.setup();
    render(
      <MoveSessionModal
        {...baseProps}
        onClose={() => {}}
        onCompleted={() => {}}
      />,
    );
    // Open the Advanced disclosure
    const summary = screen.getByText("Advanced");
    await user.click(summary);
    await user.click(
      screen.getByLabelText(/force past the live-session mtime guard/i),
    );
    await user.click(screen.getByRole("button", { name: /Move to/i }));

    await waitFor(() =>
      expect(moveSpy).toHaveBeenCalledWith(
        expect.objectContaining({ forceLive: true, forceConflict: false }),
      ),
    );
  });

  it("closes on Escape", () => {
    const onClose = vi.fn();
    render(
      <MoveSessionModal
        {...baseProps}
        onClose={onClose}
        onCompleted={() => {}}
      />,
    );
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });

  it("disables Move when the target equals the source (Other + matching input)", async () => {
    const user = userEvent.setup();
    render(
      <MoveSessionModal
        {...baseProps}
        onClose={() => {}}
        onCompleted={() => {}}
      />,
    );
    await user.selectOptions(screen.getByRole("combobox"), "__other__");
    await user.type(screen.getByPlaceholderText(/target cwd/i), "/from");
    expect(screen.getByRole("button", { name: /Move to/i })).toBeDisabled();
  });
});
