/**
 * Verifies the Knowledge section's tab strip conforms to the canonical
 * SectionTab ARIA contract (see
 * src/sections/sessions/components/SectionTab.tsx):
 *   - role=tablist / role=tab / role=tabpanel wiring via
 *     id + aria-controls + aria-labelledby
 *   - roving tabIndex (active 0, inactive -1)
 *   - Left/Right arrow keys move selection + focus with wrap-around
 *   - Dashboard is the landing view (the pane opens on health, not a list)
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const listProjectsSpy = vi.fn();
const lessonCountsSpy = vi.fn();
const lessonCountsByProjectSpy = vi.fn();

vi.mock("../api/sharedMemory", () => ({
  sharedMemoryApi: {
    search: vi.fn().mockResolvedValue({ hits: [], has_more: false }),
    readLocator: vi.fn(),
    listMemories: vi.fn().mockResolvedValue([]),
    createMemory: vi.fn(),
    archiveMemory: vi.fn(),
    listDecisions: vi.fn().mockResolvedValue([]),
    logDecision: vi.fn(),
    archiveDecision: vi.fn(),
    listEvidence: vi.fn().mockResolvedValue([]),
    memoryLinks: vi.fn().mockResolvedValue([]),
    listSessions: vi.fn().mockResolvedValue([]),
    listProjects: (...a: unknown[]) => listProjectsSpy(...a),
    lessonList: vi.fn().mockResolvedValue([]),
    lessonCounts: (...a: unknown[]) => lessonCountsSpy(...a),
    lessonCountsByProject: (...a: unknown[]) => lessonCountsByProjectSpy(...a),
    lessonAccept: vi.fn(),
    lessonReject: vi.fn(),
    recurrenceList: vi.fn().mockResolvedValue([]),
    recurrenceConfirm: vi.fn(),
    recurrenceDismiss: vi.fn(),
    recurrenceCounts: vi
      .fn()
      .mockResolvedValue({ confirmed_window: 0, pending: 0, window_days: 30 }),
  },
}));

import { SharedMemorySection } from "./SharedMemorySection";

beforeEach(() => {
  listProjectsSpy.mockResolvedValue([]);
  lessonCountsByProjectSpy.mockResolvedValue([]);
  lessonCountsSpy.mockResolvedValue({
    proposed: 0,
    accepted: 0,
    rejected: 0,
    suspect: 0,
    enforced: 0,
  });
});

describe("SharedMemorySection tabs", () => {
  it("renders the canonical tablist/tab/tabpanel contract", () => {
    render(<SharedMemorySection />);

    const tablist = screen.getByRole("tablist", { name: "Knowledge tabs" });
    expect(tablist).toBeInTheDocument();

    const tabs = screen.getAllByRole("tab");
    expect(tabs).toHaveLength(4);

    // Dashboard is the landing view — the pane opens on the state of what
    // Claude knows, never on a list.
    const dashTab = screen.getByRole("tab", { name: "Dashboard" });
    expect(dashTab).toHaveAttribute("aria-selected", "true");
    expect(dashTab).toHaveAttribute("id", "shared-memory-tab-dashboard");
    expect(dashTab).toHaveAttribute("aria-controls", "shared-memory-panel-dashboard");
    expect(dashTab).toHaveAttribute("tabindex", "0");

    for (const name of ["Know", "Review", "Recall"]) {
      const tab = screen.getByRole("tab", { name });
      expect(tab).toHaveAttribute("aria-selected", "false");
      expect(tab).toHaveAttribute("tabindex", "-1");
    }

    const panel = screen.getByRole("tabpanel");
    expect(panel).toHaveAttribute("id", "shared-memory-panel-dashboard");
    expect(panel).toHaveAttribute("aria-labelledby", "shared-memory-tab-dashboard");
  });

  it("click selects a tab and rewires the tabpanel", async () => {
    const user = userEvent.setup();
    render(<SharedMemorySection />);

    await user.click(screen.getByRole("tab", { name: "Recall" }));

    expect(screen.getByRole("tab", { name: "Recall" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    const panel = screen.getByRole("tabpanel");
    expect(panel).toHaveAttribute("id", "shared-memory-panel-recall");
    expect(
      await screen.findByText("Enter a query to search raw transcripts."),
    ).toBeInTheDocument();
  });

  it("ArrowRight moves selection and focus to the next tab", async () => {
    const user = userEvent.setup();
    render(<SharedMemorySection />);

    // Navigates from the SELECTED tab (Dashboard by default). Dashboard → Know.
    screen.getByRole("tab", { name: "Dashboard" }).focus();
    await user.keyboard("{ArrowRight}");

    const knowTab = screen.getByRole("tab", { name: "Know" });
    expect(knowTab).toHaveAttribute("aria-selected", "true");
    expect(knowTab).toHaveFocus();
  });

  it("ArrowLeft wraps from the first to the last tab", async () => {
    const user = userEvent.setup();
    render(<SharedMemorySection />);

    // Dashboard is first; ArrowLeft wraps to the last tab, Recall.
    screen.getByRole("tab", { name: "Dashboard" }).focus();
    await user.keyboard("{ArrowLeft}");

    const recallTab = screen.getByRole("tab", { name: "Recall" });
    expect(recallTab).toHaveAttribute("aria-selected", "true");
    expect(recallTab).toHaveFocus();
  });
});
