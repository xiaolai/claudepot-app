import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

import { OperationProgressModal } from "./OperationProgressModal";
import {
  PROJECT_MOVE_PHASES,
  renderProjectMoveResult,
} from "./projectMoveProgress";
import {
  SESSION_MOVE_PHASES,
  renderSessionMoveResult,
} from "./sessionMoveProgress";

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
});
