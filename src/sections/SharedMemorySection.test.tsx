/**
 * Verifies the Shared Memory section's tab strip conforms to the
 * canonical SectionTab ARIA contract (see
 * src/sections/sessions/components/SectionTab.tsx):
 *   - role=tablist / role=tab / role=tabpanel wiring via
 *     id + aria-controls + aria-labelledby
 *   - roving tabIndex (active 0, inactive -1)
 *   - Left/Right arrow keys move selection + focus with wrap-around
 *     (the companion that keeps inactive tabs keyboard-reachable)
 *   - one primary action: while the Memories create form is open,
 *     "Save" is the only solid button.
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const searchSpy = vi.fn();
const listMemoriesSpy = vi.fn();
const listDecisionsSpy = vi.fn();
const lessonListSpy = vi.fn();
const lessonCountsSpy = vi.fn();

vi.mock("../api/sharedMemory", () => ({
  sharedMemoryApi: {
    search: (...a: unknown[]) => searchSpy(...a),
    readLocator: vi.fn(),
    listMemories: (...a: unknown[]) => listMemoriesSpy(...a),
    createMemory: vi.fn(),
    archiveMemory: vi.fn(),
    listDecisions: (...a: unknown[]) => listDecisionsSpy(...a),
    archiveDecision: vi.fn(),
    lessonList: (...a: unknown[]) => lessonListSpy(...a),
    lessonCounts: (...a: unknown[]) => lessonCountsSpy(...a),
    lessonAccept: vi.fn(),
    lessonReject: vi.fn(),
  },
}));

import { SharedMemorySection } from "./SharedMemorySection";

beforeEach(() => {
  searchSpy.mockResolvedValue({ hits: [], has_more: false });
  listMemoriesSpy.mockResolvedValue([]);
  listDecisionsSpy.mockResolvedValue([]);
  lessonListSpy.mockResolvedValue([]);
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

    const tablist = screen.getByRole("tablist", {
      name: "Shared memory tabs",
    });
    expect(tablist).toBeInTheDocument();

    const tabs = screen.getAllByRole("tab");
    expect(tabs).toHaveLength(4);

    // Lessons is the default landing tab — the triage queue is the
    // primary verb of this section now.
    const lessonsTab = screen.getByRole("tab", { name: "Lessons" });
    expect(lessonsTab).toHaveAttribute("aria-selected", "true");
    expect(lessonsTab).toHaveAttribute("id", "shared-memory-tab-lessons");
    expect(lessonsTab).toHaveAttribute(
      "aria-controls",
      "shared-memory-panel-lessons",
    );
    expect(lessonsTab).toHaveAttribute("tabindex", "0");

    // Inactive tabs leave the tab order (roving tabIndex).
    for (const name of ["Search", "Memories", "Decisions"]) {
      const tab = screen.getByRole("tab", { name });
      expect(tab).toHaveAttribute("aria-selected", "false");
      expect(tab).toHaveAttribute("tabindex", "-1");
    }

    const panel = screen.getByRole("tabpanel");
    expect(panel).toHaveAttribute("id", "shared-memory-panel-lessons");
    expect(panel).toHaveAttribute(
      "aria-labelledby",
      "shared-memory-tab-lessons",
    );
  });

  it("click selects a tab and rewires the tabpanel", async () => {
    const user = userEvent.setup();
    render(<SharedMemorySection />);

    await user.click(screen.getByRole("tab", { name: "Memories" }));

    expect(
      screen.getByRole("tab", { name: "Memories" }),
    ).toHaveAttribute("aria-selected", "true");
    const panel = screen.getByRole("tabpanel");
    expect(panel).toHaveAttribute("id", "shared-memory-panel-memories");
    expect(panel).toHaveAttribute(
      "aria-labelledby",
      "shared-memory-tab-memories",
    );
    expect(await screen.findByText("No memories yet.")).toBeInTheDocument();
  });

  it("ArrowRight moves selection and focus to the next tab", async () => {
    const user = userEvent.setup();
    render(<SharedMemorySection />);

    // The handler navigates from the SELECTED tab (Lessons by default),
    // not from document focus. Lessons → Search.
    screen.getByRole("tab", { name: "Lessons" }).focus();
    await user.keyboard("{ArrowRight}");

    const searchTab = screen.getByRole("tab", { name: "Search" });
    expect(searchTab).toHaveAttribute("aria-selected", "true");
    expect(searchTab).toHaveFocus();
  });

  it("ArrowLeft wraps from the first to the last tab", async () => {
    const user = userEvent.setup();
    render(<SharedMemorySection />);

    // Lessons is first; ArrowLeft wraps to the last tab, Decisions.
    screen.getByRole("tab", { name: "Lessons" }).focus();
    await user.keyboard("{ArrowLeft}");

    const decisionsTab = screen.getByRole("tab", { name: "Decisions" });
    expect(decisionsTab).toHaveAttribute("aria-selected", "true");
    expect(decisionsTab).toHaveFocus();
    expect(listDecisionsSpy).toHaveBeenCalled();
  });

  it("keeps one solid primary action while the create form is open", async () => {
    const user = userEvent.setup();
    render(<SharedMemorySection />);

    await user.click(screen.getByRole("tab", { name: "Memories" }));
    await user.click(
      await screen.findByRole("button", { name: /Add memory/ }),
    );

    // Form is open — "Save" exists and is the lone solid (the solid
    // Button variant paints `background: var(--accent)` inline).
    const save = screen.getByRole("button", { name: "Save" });
    const solidButtons = screen
      .getAllByRole("button")
      .filter((b) =>
        (b.getAttribute("style") ?? "").includes("var(--accent)"),
      );
    expect(solidButtons).toEqual([save]);
  });
});
