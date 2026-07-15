/**
 * Phase 1 — the Dashboard landing view.
 *
 * The reframe made testable: the four signals lead, "stored" is never a
 * hero, the empty state points at the harvest command, and the coverage
 * grid deep-links into Know. Plus the merge/sort that surfaces the
 * most-sessions-least-curated project.
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const lessonCountsSpy = vi.fn();
const lessonCountsByProjectSpy = vi.fn();
const listProjectsSpy = vi.fn();
const recurrenceCountsSpy = vi.fn();

vi.mock("../../api/sharedMemory", () => ({
  sharedMemoryApi: {
    lessonCounts: (...a: unknown[]) => lessonCountsSpy(...a),
    lessonCountsByProject: (...a: unknown[]) => lessonCountsByProjectSpy(...a),
    listProjects: (...a: unknown[]) => listProjectsSpy(...a),
    recurrenceCounts: (...a: unknown[]) => recurrenceCountsSpy(...a),
  },
}));

import { KnowledgeDashboard, mergeCoverage } from "./KnowledgeDashboard";

const ZERO = { proposed: 0, accepted: 0, rejected: 0, suspect: 0, enforced: 0 };

beforeEach(() => {
  lessonCountsSpy.mockResolvedValue(ZERO);
  lessonCountsByProjectSpy.mockResolvedValue([]);
  listProjectsSpy.mockResolvedValue([]);
  recurrenceCountsSpy.mockResolvedValue({
    confirmed_window: 0,
    pending: 0,
    window_days: 30,
  });
});

describe("KnowledgeDashboard", () => {
  it("leads with the four signals and never shows 'stored' as a hero", async () => {
    lessonCountsSpy.mockResolvedValue({
      proposed: 2,
      accepted: 3,
      rejected: 1,
      suspect: 1,
      enforced: 1,
    });
    render(<KnowledgeDashboard onOpenProject={vi.fn()} />);

    expect(await screen.findByText("Recurrence")).toBeInTheDocument();
    expect(screen.getByText("Suspect")).toBeInTheDocument();
    expect(screen.getByText("Enforced")).toBeInTheDocument();
    expect(screen.getByText("Coverage")).toBeInTheDocument();

    // The vanity metric is banished — the word never appears.
    expect(screen.queryByText(/stored/i)).toBeNull();
  });

  it("surfaces the confirmed-recurrence count and points pending ones at Review", async () => {
    recurrenceCountsSpy.mockResolvedValue({
      confirmed_window: 2,
      pending: 3,
      window_days: 30,
    });
    render(<KnowledgeDashboard onOpenProject={vi.fn()} />);
    // The headline number.
    expect(await screen.findByText("2")).toBeInTheDocument();
    // Pending candidates route the user to Review, not the count.
    expect(
      screen.getByText("3 awaiting confirmation in Review"),
    ).toBeInTheDocument();
  });

  it("empty state points at the harvest command", async () => {
    render(<KnowledgeDashboard onOpenProject={vi.fn()} />);
    expect(await screen.findByText("claudepot lesson harvest")).toBeInTheDocument();
  });

  it("clicking a coverage row deep-links into Know for that project", async () => {
    const onOpen = vi.fn();
    listProjectsSpy.mockResolvedValue([
      { project_path: "/proj/alpha", session_count: 12, last_activity_ms: null },
    ]);
    lessonCountsByProjectSpy.mockResolvedValue([
      { project_path: "/proj/alpha", counts: { ...ZERO, accepted: 1 } },
    ]);
    const user = userEvent.setup();
    render(<KnowledgeDashboard onOpenProject={onOpen} />);

    const row = await screen.findByRole("button", { name: /alpha/ });
    await user.click(row);
    expect(onOpen).toHaveBeenCalledWith("/proj/alpha");
  });
});

describe("mergeCoverage", () => {
  it("sorts most-sessions-least-curated first", () => {
    const rows = mergeCoverage(
      [{ project_path: "/curated", counts: { ...ZERO, accepted: 5 } }],
      [
        { project_path: "/curated", session_count: 50, last_activity_ms: null },
        { project_path: "/busy-unmined", session_count: 40, last_activity_ms: null },
        { project_path: "/quiet-unmined", session_count: 3, last_activity_ms: null },
      ],
    );
    // Uncurated projects float above the curated one even though it has
    // the most sessions; within uncurated, most sessions first.
    expect(rows.map((r) => r.projectPath)).toEqual([
      "/busy-unmined",
      "/quiet-unmined",
      "/curated",
    ]);
  });

  it("includes projects that have memories but no indexed sessions", () => {
    const rows = mergeCoverage(
      [{ project_path: "/orphan", counts: { ...ZERO, accepted: 1 } }],
      [],
    );
    expect(rows).toHaveLength(1);
    expect(rows[0]!.sessionCount).toBe(0);
    expect(rows[0]!.curated).toBe(1);
  });
});
