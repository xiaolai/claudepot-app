/**
 * Phase 1 — the Dashboard landing view.
 *
 * The reframe made testable: the pane leads with *open work* (what needs
 * attention, each routing to its action), "stored" is never a hero, a
 * failed load never masquerades as a healthy zero, and the coverage grid
 * deep-links into Know. Plus the merge/sort that surfaces the
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
const NO_RECUR = { confirmed_window: 0, pending: 0, window_days: 30 };

beforeEach(() => {
  lessonCountsSpy.mockReset().mockResolvedValue(ZERO);
  lessonCountsByProjectSpy.mockReset().mockResolvedValue([]);
  listProjectsSpy.mockReset().mockResolvedValue([]);
  recurrenceCountsSpy.mockReset().mockResolvedValue(NO_RECUR);
});

describe("KnowledgeDashboard", () => {
  it("leads with open work and never shows 'stored' as a hero", async () => {
    lessonCountsSpy.mockResolvedValue({
      proposed: 2,
      accepted: 3,
      rejected: 1,
      suspect: 1,
      enforced: 1,
    });
    render(<KnowledgeDashboard onOpenProject={vi.fn()} onOpenReview={vi.fn()} />);

    expect(await screen.findByText("Needs attention")).toBeInTheDocument();
    // Open-work heroes, each an action, not a stored count.
    expect(screen.getByText("Suspect")).toBeInTheDocument();
    expect(screen.getByText("Proposals")).toBeInTheDocument();
    // Trust scoreboard is demoted to a context line, not a hero.
    expect(screen.getByText(/1 enforced · 2 documented/)).toBeInTheDocument();
    // The vanity metric is banished — the word never appears.
    expect(screen.queryByText(/stored/i)).toBeNull();
  });

  it("leads with pending recurrences and routes them to Review", async () => {
    lessonCountsSpy.mockResolvedValue({ ...ZERO, accepted: 2 });
    recurrenceCountsSpy.mockResolvedValue({
      confirmed_window: 2,
      pending: 3,
      window_days: 30,
    });
    const onOpenReview = vi.fn();
    const user = userEvent.setup();
    render(<KnowledgeDashboard onOpenProject={vi.fn()} onOpenReview={onOpenReview} />);

    // The pending count leads (it is the thing to act on), not the
    // historical confirmed total.
    const card = await screen.findByRole("button", { name: /Recurrences/ });
    expect(card).toHaveTextContent("3");
    // The confirmed-in-window total is honest context, not a green hero.
    expect(screen.getByText(/2 confirmed repeats \(30d\)/)).toBeInTheDocument();

    await user.click(card);
    expect(onOpenReview).toHaveBeenCalled();
  });

  it("a suspect hero routes to the suspect queue", async () => {
    lessonCountsSpy.mockResolvedValue({ ...ZERO, accepted: 4, suspect: 2 });
    const onOpenReview = vi.fn();
    const user = userEvent.setup();
    render(<KnowledgeDashboard onOpenProject={vi.fn()} onOpenReview={onOpenReview} />);

    await user.click(await screen.findByRole("button", { name: /Suspect/ }));
    expect(onOpenReview).toHaveBeenCalledWith("suspect");
  });

  it("shows 'all caught up' when there is knowledge but no open work", async () => {
    lessonCountsSpy.mockResolvedValue({ ...ZERO, accepted: 3, enforced: 1 });
    render(<KnowledgeDashboard onOpenProject={vi.fn()} onOpenReview={vi.fn()} />);
    expect(await screen.findByText("All caught up.")).toBeInTheDocument();
  });

  it("empty state points at the harvest command", async () => {
    render(<KnowledgeDashboard onOpenProject={vi.fn()} onOpenReview={vi.fn()} />);
    expect(await screen.findByText("claudepot lesson harvest")).toBeInTheDocument();
  });

  it("a total load failure shows an error and a retry, not a green zero", async () => {
    lessonCountsSpy.mockRejectedValue("boom");
    lessonCountsByProjectSpy.mockRejectedValue("boom");
    listProjectsSpy.mockRejectedValue("boom");
    recurrenceCountsSpy.mockRejectedValue("boom");
    render(<KnowledgeDashboard onOpenProject={vi.fn()} onOpenReview={vi.fn()} />);

    expect(await screen.findByRole("button", { name: "Retry" })).toBeInTheDocument();
    // The reassuring lies are suppressed on total failure.
    expect(screen.queryByText("All caught up.")).toBeNull();
    expect(screen.queryByText("claudepot lesson harvest")).toBeNull();
  });

  it("a partial failure still renders what loaded", async () => {
    lessonCountsSpy.mockResolvedValue({ ...ZERO, accepted: 2, suspect: 1 });
    recurrenceCountsSpy.mockRejectedValue("recurrence table missing");
    render(<KnowledgeDashboard onOpenProject={vi.fn()} onOpenReview={vi.fn()} />);

    // Suspect (from the call that succeeded) still shows...
    expect(await screen.findByText("Suspect")).toBeInTheDocument();
    // ...alongside a non-fatal partial warning.
    expect(screen.getByText(/couldn't load/)).toBeInTheDocument();
  });

  it("never claims 'no known failure has recurred' when the recurrence call failed", async () => {
    // Knowledge present, no open work, but recurrence — the very fact the
    // caught-up copy would assert — failed to load.
    lessonCountsSpy.mockResolvedValue({ ...ZERO, accepted: 3 });
    recurrenceCountsSpy.mockRejectedValue("recurrence table missing");
    render(<KnowledgeDashboard onOpenProject={vi.fn()} onOpenReview={vi.fn()} />);

    expect(
      await screen.findByText(/Recurrence status couldn't be loaded/),
    ).toBeInTheDocument();
    expect(screen.queryByText(/no known failure has recurred/)).toBeNull();
  });

  it("a counts failure suppresses the fabricated cold-start lead", async () => {
    // lessonCounts fails; projects load. The old code coerced counts to zero
    // and showed "No lessons yet" — a fabricated cold start.
    lessonCountsSpy.mockRejectedValue("counts down");
    listProjectsSpy.mockResolvedValue([
      { project_path: "/proj/busy", session_count: 9, last_activity_ms: null },
    ]);
    render(<KnowledgeDashboard onOpenProject={vi.fn()} onOpenReview={vi.fn()} />);

    // Partial banner shows; the false cold-start does not; the grid still renders.
    expect(await screen.findByText(/couldn't load/)).toBeInTheDocument();
    expect(screen.queryByText(/No lessons yet/)).toBeNull();
    expect(screen.getByText(/9 session/)).toBeInTheDocument();
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
    render(<KnowledgeDashboard onOpenProject={onOpen} onOpenReview={vi.fn()} />);

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
