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

// Capture the subscribed handlers so a test can deliver a real event
// on the channel the modal actually listens to. Previously this dropped
// them, which is why nothing covered "an event and a poll disagree".
const bus = vi.hoisted(() => new Map<string, (e: unknown) => void>());
vi.mock("@tauri-apps/api/event", () => ({
  listen: (channel: string, handler: (e: unknown) => void) => {
    bus.set(channel, handler);
    return Promise.resolve(() => bus.delete(channel));
  },
}));

/** Deliver an `op-progress::<op_id>` event exactly as the backend would. */
function emit(payload: {
  op_id: string;
  phase: string;
  status: "running" | "complete" | "error";
  done?: number;
  total?: number;
  detail?: string;
}) {
  bus.get(`op-progress::${payload.op_id}`)?.({ payload });
}

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
      phase_states: {},
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

  it("seeds phases that finished before it mounted", async () => {
    // The observed bug: a session move's S1 and S2 complete before this
    // modal subscribes, so their events go to nobody and both rows read
    // "Pending" for the rest of the op — next to an S3 that is clearly
    // past them. The phase list was lying about work already done.
    vi.useFakeTimers();
    try {
      renderWithBackstop(async () =>
        mkInfo({
          status: "running",
          current_phase: "S3",
          phase_states: { S1: "complete", S2: "complete", S3: "running" },
        }),
      );
      // The very first poll is immediate — waiting a full interval to
      // correct the rows is the visible half of the bug.
      await advance(50);

      const rows = [...document.querySelectorAll(".phase-list li")].map(
        (li) => li.textContent ?? "",
      );
      expect(rows[0]).toMatch(/Rewriting primary transcript.*complete/);
      expect(rows[1]).toMatch(/Moving sidecar dirs.*complete/);
      expect(rows[2]).toMatch(/Updating history\.jsonl.*running/);
      // Phases nobody has reported stay pending — seeding fills gaps,
      // it does not invent progress.
      expect(rows[3]).toMatch(/Clearing \.claude\.json pointers.*pending/);
    } finally {
      vi.useRealTimers();
    }
  });

  it("a live event beats a stale poll for the same phase", async () => {
    // Events are the transport and are newer than a 2 s poll. Seeding
    // must fill gaps only — never walk a phase backwards.
    vi.useFakeTimers();
    try {
      renderWithBackstop(async () =>
        mkInfo({ status: "running", phase_states: { S1: "running" } }),
      );
      await advance(50);
      expect(
        [...document.querySelectorAll(".phase-list li")][0]?.textContent,
      ).toMatch(/Rewriting primary transcript.*running/);

      // The phase completes on the live channel...
      act(() => {
        emit({ op_id: "op-backstop", phase: "S1", status: "complete" });
      });
      expect(
        [...document.querySelectorAll(".phase-list li")][0]?.textContent,
      ).toMatch(/Rewriting primary transcript.*complete/);

      // ...and a later poll still reporting "running" must not undo it.
      await advance(2100);
      expect(
        [...document.querySelectorAll(".phase-list li")][0]?.textContent,
      ).toMatch(/Rewriting primary transcript.*complete/);
    } finally {
      vi.useRealTimers();
    }
  });

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

  it("the mount poll never concludes the op is untracked", async () => {
    // "Not found on the very first look" is a startup race, not proof
    // the op ended. Announcing an unknown outcome the instant the
    // dialog opens would be its own lie — and it would hide the Cancel
    // button on every op that offers one.
    vi.useFakeTimers();
    try {
      const onComplete = vi.fn();
      renderWithBackstop(async () => null, { onComplete });
      await advance(50);
      expect(screen.queryByText("Finished — outcome unknown.")).toBeNull();

      // One interval later, absence does mean something.
      await advance(2100);
      expect(
        screen.getByText("Finished — outcome unknown."),
      ).toBeInTheDocument();
      expect(onComplete).not.toHaveBeenCalled();
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
