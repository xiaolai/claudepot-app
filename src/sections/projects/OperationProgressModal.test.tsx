import { describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { OperationProgressModal } from "./OperationProgressModal";
import {
  PROJECT_MOVE_PHASES,
  renderProjectMoveResult,
} from "./projectMoveProgress";
import {
  SESSION_MOVE_PHASES,
  renderSessionMoveResult,
} from "./sessionMoveProgress";
import type { RunningOpInfo } from "../../types";

vi.mock("@tauri-apps/api/event", () => ({
  // The hook calls listen() and never gets to fire its handler in
  // tests — we only care about the static render. Resolve a no-op
  // unlisten so the hook doesn't throw.
  listen: () => Promise.resolve(() => {}),
}));

describe("OperationProgressModal", () => {
  it("renders every project-move phase label in order", () => {
    render(
      <OperationProgressModal
        opId="op-pm"
        title="Renaming foo → bar"
        phases={PROJECT_MOVE_PHASES}
        fetchStatus={async () => null}
        renderResult={renderProjectMoveResult}
        onClose={() => {}}
      />,
    );
    for (const phase of PROJECT_MOVE_PHASES) {
      expect(screen.getByText(phase.label)).toBeInTheDocument();
    }
    // Phase 6 carries the dynamic label so we can spot-check it
    // appears as text rather than as the bare phase id.
    expect(screen.queryByText("P6")).toBeNull();
  });

  it("renders every session-move phase label in order", () => {
    render(
      <OperationProgressModal
        opId="op-sm"
        title="Moving session abcdef01 → main"
        phases={SESSION_MOVE_PHASES}
        fetchStatus={async () => null}
        renderResult={renderSessionMoveResult}
        onClose={() => {}}
      />,
    );
    for (const phase of SESSION_MOVE_PHASES) {
      expect(screen.getByText(phase.label)).toBeInTheDocument();
    }
    // Internal id is a tooltip, not visible text.
    expect(screen.queryByText("S1")).toBeNull();
  });

  it("renders the title in the header", () => {
    render(
      <OperationProgressModal
        opId="op-title"
        title="Test op title"
        phases={SESSION_MOVE_PHASES}
        fetchStatus={async () => null}
        onClose={() => {}}
      />,
    );
    expect(screen.getByText("Test op title")).toBeInTheDocument();
  });

  it("renders no Cancel button when onCancel is omitted", () => {
    render(
      <OperationProgressModal
        opId="op-no-cancel"
        title="Renaming"
        phases={PROJECT_MOVE_PHASES}
        fetchStatus={async () => null}
        onClose={() => {}}
      />,
    );
    expect(
      screen.queryByRole("button", { name: /cancel/i }),
    ).not.toBeInTheDocument();
  });

  it("renders the Cancel button and invokes onCancel on click", async () => {
    const onCancel = vi.fn();
    render(
      <OperationProgressModal
        opId="op-cancel"
        title="Re-login: alice@example.com"
        phases={PROJECT_MOVE_PHASES}
        fetchStatus={async () => null}
        onClose={() => {}}
        onCancel={onCancel}
        cancelLabel="Cancel login"
      />,
    );
    const btn = screen.getByRole("button", { name: "Cancel login" });
    await userEvent.click(btn);
    expect(onCancel).toHaveBeenCalledOnce();
  });

  // ── Polling backstop ──────────────────────────────────────────
  //
  // Every terminal state depends on one `op` event arriving. When it
  // doesn't — emitted while the webview drains a backlog, listener a
  // beat late — this modal used to wait forever over a scrim, which is
  // the "frozen dialog" a real user hit on a session move. These pin
  // that `RunningOps` is consulted instead.

  /** Advance fake timers inside act() so the state update the poll
   *  resolves into is flushed before we assert on the DOM. */
  async function advance(ms: number) {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(ms);
    });
  }

  function mkInfo(over: Partial<RunningOpInfo> = {}): RunningOpInfo {
    return {
      op_id: "op-backstop",
      kind: "session_move",
      old_path: "/from",
      new_path: "/to",
      current_phase: "S3",
      sub_progress: null,
      status: "running",
      started_unix_secs: 0,
      last_error: null,
      move_result: null,
      clean_result: null,
      failed_journal_id: null,
      ...over,
    };
  }

  /** Render with fake timers so the 2 s backstop is drivable. */
  function renderWithBackstop(
    fetchStatus: (opId: string) => Promise<RunningOpInfo | null>,
    handlers: {
      onComplete?: () => void;
      onError?: (d: string | null) => void;
    } = {},
  ) {
    return render(
      <OperationProgressModal
        opId="op-backstop"
        title="Moving session"
        phases={SESSION_MOVE_PHASES}
        fetchStatus={fetchStatus}
        onClose={() => {}}
        {...handlers}
      />,
    );
  }

  it("reaches a terminal state from polling when the event never arrives", async () => {
    vi.useFakeTimers();
    try {
      const onComplete = vi.fn();
      renderWithBackstop(async () => mkInfo({ status: "complete" }), {
        onComplete,
      });
      expect(screen.queryByText(/Complete/)).toBeNull();

      await advance(2100);

      expect(screen.getByText("✓ Complete.")).toBeInTheDocument();
      expect(onComplete).toHaveBeenCalledOnce();
    } finally {
      vi.useRealTimers();
    }
  });

  it("surfaces a polled error with its detail", async () => {
    vi.useFakeTimers();
    try {
      const onError = vi.fn();
      renderWithBackstop(
        async () =>
          mkInfo({ status: "error", last_error: "target slug collision" }),
        { onError },
      );

      await advance(2100);

      expect(screen.getByText("Error.")).toBeInTheDocument();
      expect(screen.getByText("target slug collision")).toBeInTheDocument();
      expect(onError).toHaveBeenCalledWith("target slug collision");
    } finally {
      vi.useRealTimers();
    }
  });

  it("says the outcome is unknown when the op already left RunningOps", async () => {
    // `remove_after_grace` drops the op 5 s after it ends. Landing here
    // means we cannot know whether it succeeded — claiming either way
    // would be a guess the caller acts on.
    vi.useFakeTimers();
    try {
      const onComplete = vi.fn();
      const onError = vi.fn();
      renderWithBackstop(async () => null, { onComplete, onError });

      await advance(2100);

      expect(screen.getByText("Finished — outcome unknown.")).toBeInTheDocument();
      expect(onComplete).not.toHaveBeenCalled();
      expect(onError).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("settles once and stops polling", async () => {
    vi.useFakeTimers();
    try {
      const onComplete = vi.fn();
      const fetchStatus = vi.fn(async () => mkInfo({ status: "complete" }));
      renderWithBackstop(fetchStatus, { onComplete });

      await advance(2100);
      const callsAtSettle = fetchStatus.mock.calls.length;
      await advance(10_000);

      expect(onComplete).toHaveBeenCalledOnce();
      expect(fetchStatus).toHaveBeenCalledTimes(callsAtSettle);
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps waiting while the op is still running", async () => {
    vi.useFakeTimers();
    try {
      const onComplete = vi.fn();
      renderWithBackstop(async () => mkInfo({ status: "running" }), {
        onComplete,
      });

      await advance(10_000);

      expect(screen.queryByText("✓ Complete.")).toBeNull();
      expect(screen.queryByText("Finished — outcome unknown.")).toBeNull();
      expect(onComplete).not.toHaveBeenCalled();
      // Still showing the live phase list, not a terminal card.
      expect(screen.getByText("Updating history.jsonl")).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("a failed poll is not treated as a terminal state", async () => {
    vi.useFakeTimers();
    try {
      const onComplete = vi.fn();
      const onError = vi.fn();
      renderWithBackstop(async () => {
        throw new Error("IPC unavailable");
      }, { onComplete, onError });

      await advance(10_000);

      expect(onComplete).not.toHaveBeenCalled();
      expect(onError).not.toHaveBeenCalled();
      expect(screen.queryByText("Error.")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });
});
