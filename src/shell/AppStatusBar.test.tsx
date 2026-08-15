import {
  afterEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { AppStatusBar, formatLiveSegment } from "./AppStatusBar";
import type { LiveSessionSummary } from "../types";

function mkSession(overrides: Partial<LiveSessionSummary> = {}): LiveSessionSummary {
  return {
    session_id: overrides.session_id ?? "s",
    pid: overrides.pid ?? 1,
    cwd: overrides.cwd ?? "/tmp/p",
    transcript_path: null,
    status: overrides.status ?? "busy",
    current_action: null,
    model: overrides.model ?? null,
    waiting_for: null,
    errored: false,
    stuck: false,
    idle_ms: 0,
    seq: 0,
  };
}

describe("AppStatusBar helpers", () => {
  describe("formatLiveSegment", () => {
    it("returns null when no sessions are live", () => {
      expect(formatLiveSegment([])).toBeNull();
    });

    it("renders a count and nothing else", () => {
      expect(formatLiveSegment([mkSession({ model: null })])).toBe("● 1 live");
    });

    /**
     * The model mix (`· OPUS 2, SON 1`) used to ride along here.
     *
     * Five surfaces rendered the same `useSessionLive` data and none of
     * them said which was authoritative, so each now has one job:
     * status bar = the count, sidebar strip = the list, Activities =
     * history and cost. The bar's own comment had already conceded the
     * mix "reads as opaque jargon to a new user".
     */
    it("does NOT append a model mix", () => {
      const sessions = [
        mkSession({ model: "claude-opus-4-7" }),
        mkSession({ model: "claude-opus-4-7" }),
        mkSession({ model: "claude-sonnet-4-6" }),
      ];
      expect(formatLiveSegment(sessions)).toBe("● 3 live");
    });

    it("counts unknown-model sessions like any other", () => {
      const sessions = [
        mkSession({ model: null }),
        mkSession({ model: "claude-opus-4-7" }),
      ];
      expect(formatLiveSegment(sessions)).toBe("● 2 live");
    });
  });
});


vi.mock("../hooks/useSessionLive", () => ({
  useSessionLive: () => [],
}));


/**
 * Right-cluster chip rendering. Locks down the contract: the chips
 * appear when their hooks have nonzero data, are wired to the
 * corresponding callbacks, and disappear when handlers are absent.
 */
describe("AppStatusBar — chip rendering", () => {
  afterEach(() => cleanup());

  const stats = { projects: null, sessions: null };

  it("hides the running-ops chip when the list is empty", () => {
    render(
      <AppStatusBar
        stats={stats}
        runningOps={[]}
        onReopenOp={() => {}}
      />,
    );
    expect(screen.queryByText(/op$/)).toBeNull();
  });

  it("renders the running-ops chip with a singular label", () => {
    render(
      <AppStatusBar
        stats={stats}
        runningOps={[
          {
            op_id: "op-1",
            kind: "verify_all",
            old_path: "",
            new_path: "",
            current_phase: null,
            phase_states: {},
            sub_progress: null,
            status: "running",
            started_unix_secs: 0,
            last_error: null,
            move_result: null,
            clean_result: null,
            failed_journal_id: null,
          },
        ]}
        onReopenOp={() => {}}
      />,
    );
    expect(screen.getByText("1 op")).toBeInTheDocument();
  });

  it("hides the pending chip when summary is null", () => {
    render(
      <AppStatusBar
        stats={stats}
        pendingSummary={null}
        onOpenRepair={() => {}}
      />,
    );
    expect(screen.queryByText(/pending$/)).toBeNull();
  });

  it("renders the pending chip with the count and warn tone for stale", () => {
    render(
      <AppStatusBar
        stats={stats}
        pendingSummary={{ pending: 1, stale: 2, running: 0 }}
        onOpenRepair={() => {}}
      />,
    );
    const chip = screen.getByText("3 pending").closest("button");
    expect(chip?.className).toContain("warn");
  });
});
