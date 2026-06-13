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

vi.mock("../api/sharedMemory", () => ({
  sharedMemoryApi: {
    search: (...a: unknown[]) => searchSpy(...a),
    readLocator: vi.fn(),
    listMemories: (...a: unknown[]) => listMemoriesSpy(...a),
    createMemory: vi.fn(),
    archiveMemory: vi.fn(),
    listDecisions: (...a: unknown[]) => listDecisionsSpy(...a),
    archiveDecision: vi.fn(),
  },
}));

import { SharedMemorySection } from "./SharedMemorySection";

beforeEach(() => {
  searchSpy.mockResolvedValue({ hits: [], has_more: false });
  listMemoriesSpy.mockResolvedValue([]);
  listDecisionsSpy.mockResolvedValue([]);
});

describe("SharedMemorySection tabs", () => {
  it("renders the canonical tablist/tab/tabpanel contract", () => {
    render(<SharedMemorySection />);

    const tablist = screen.getByRole("tablist", {
      name: "Shared memory tabs",
    });
    expect(tablist).toBeInTheDocument();

    const tabs = screen.getAllByRole("tab");
    expect(tabs).toHaveLength(3);

    const searchTab = screen.getByRole("tab", { name: "Search" });
    expect(searchTab).toHaveAttribute("aria-selected", "true");
    expect(searchTab).toHaveAttribute("id", "shared-memory-tab-search");
    expect(searchTab).toHaveAttribute(
      "aria-controls",
      "shared-memory-panel-search",
    );
    expect(searchTab).toHaveAttribute("tabindex", "0");

    // Inactive tabs leave the tab order (roving tabIndex).
    for (const name of ["Memories", "Decisions"]) {
      const tab = screen.getByRole("tab", { name });
      expect(tab).toHaveAttribute("aria-selected", "false");
      expect(tab).toHaveAttribute("tabindex", "-1");
    }

    const panel = screen.getByRole("tabpanel");
    expect(panel).toHaveAttribute("id", "shared-memory-panel-search");
    expect(panel).toHaveAttribute(
      "aria-labelledby",
      "shared-memory-tab-search",
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

    screen.getByRole("tab", { name: "Search" }).focus();
    await user.keyboard("{ArrowRight}");

    const memoriesTab = screen.getByRole("tab", { name: "Memories" });
    expect(memoriesTab).toHaveAttribute("aria-selected", "true");
    expect(memoriesTab).toHaveFocus();
    expect(listMemoriesSpy).toHaveBeenCalled();
  });

  it("ArrowLeft wraps from the first to the last tab", async () => {
    const user = userEvent.setup();
    render(<SharedMemorySection />);

    screen.getByRole("tab", { name: "Search" }).focus();
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
