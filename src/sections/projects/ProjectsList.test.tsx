import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { ProjectsList } from "./ProjectsList";
import type { ProjectInfo } from "../../types";

function mk(overrides: Partial<ProjectInfo>): ProjectInfo {
  return {
    sanitized_name: "-tmp-x",
    original_path: "/tmp/x",
    session_count: 1,
    memory_file_count: 0,
    total_size_bytes: 100,
    last_modified_ms: null,
    is_orphan: false,
    is_reachable: true,
    is_empty: false,
    ...overrides,
  };
}

describe("ProjectsList", () => {
  it("renders distinct status badges for orphan / unreachable / empty", () => {
    const projects: ProjectInfo[] = [
      mk({ sanitized_name: "-live", original_path: "/live" }),
      mk({ sanitized_name: "-gone", original_path: "/deleted", is_orphan: true, is_reachable: true }),
      mk({ sanitized_name: "-unmounted", original_path: "/Volumes/ext/proj", is_orphan: false, is_reachable: false }),
      mk({ sanitized_name: "-abandoned", original_path: "/tmp/abandoned", is_orphan: true, is_empty: true, session_count: 0 }),
    ];

    render(
      <ProjectsList
        projects={projects}
        selectedPath={null}
        onSelect={() => {}}
        filter="all"
        onFilterChange={() => {}}
      />,
    );

    expect(screen.getByLabelText(/orphan — source dir missing/)).toBeInTheDocument();
    expect(screen.getByLabelText(/unreachable — mount the source volume/)).toBeInTheDocument();
    expect(screen.getByLabelText(/empty — CC project dir has no content/)).toBeInTheDocument();
  });

  it("filter chips report accurate counts", () => {
    const projects: ProjectInfo[] = [
      mk({ sanitized_name: "a", original_path: "/a" }),
      mk({ sanitized_name: "b", original_path: "/b", is_orphan: true }),
      mk({ sanitized_name: "c", original_path: "/c", is_orphan: true }),
      mk({ sanitized_name: "d", original_path: "/Volumes/ext/d", is_reachable: false }),
    ];

    render(
      <ProjectsList
        projects={projects}
        selectedPath={null}
        onSelect={() => {}}
        filter="all"
        onFilterChange={() => {}}
      />,
    );

    // Filter chips are toggle buttons inside a role=toolbar — not
    // tabs, since the list pane is always visible regardless of which
    // chip is selected (a toggled chip filters an existing panel,
    // rather than swapping between disjoint panels as tabs do).
    const orphanChip = screen.getByRole("button", { name: /Orphan/ });
    expect(orphanChip.textContent).toMatch(/2/);

    const unreachableChip = screen.getByRole("button", { name: /Unreachable/ });
    expect(unreachableChip.textContent).toMatch(/1/);

    const emptyChip = screen.getByRole("button", { name: /Empty/ });
    expect(emptyChip.textContent).not.toMatch(/\d/);
    const allChip = screen.getByRole("button", { name: /All projects/ });
    expect(allChip.textContent).toMatch(/4/);
  });

  it("invokes onFilterChange when a chip is clicked", async () => {
    const user = userEvent.setup();
    const spy = vi.fn();
    render(
      <ProjectsList
        projects={[mk({ is_orphan: true })]}
        selectedPath={null}
        onSelect={() => {}}
        filter="all"
        onFilterChange={spy}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Orphan/ }));
    expect(spy).toHaveBeenCalledWith("orphan");
  });

  it("fires onContextMenu with the project when a row is right-clicked", () => {
    const spy = vi.fn();
    const project = mk({
      sanitized_name: "-click-me",
      original_path: "/tmp/click-me",
    });
    render(
      <ProjectsList
        projects={[project]}
        selectedPath={null}
        onSelect={() => {}}
        onContextMenu={spy}
        filter="all"
        onFilterChange={() => {}}
      />,
    );
    const row = screen.getByRole("option", { name: /click-me/ });
    // fireEvent.contextMenu bypasses the pointer-event stack userEvent
    // would require and maps cleanly to React's onContextMenu handler.
    fireEvent.contextMenu(row);
    expect(spy).toHaveBeenCalledTimes(1);
    // Second arg is the ProjectInfo; assert by original_path to keep
    // the test robust against shape changes that don't affect behavior.
    expect(spy.mock.calls[0][1].original_path).toBe("/tmp/click-me");
  });

  it("omits onContextMenu handler when no callback is provided", () => {
    // Regression guard: the row must not throw when the optional prop
    // is omitted (the detail pane may be rendered outside a section
    // that wires context menus).
    render(
      <ProjectsList
        projects={[mk({ sanitized_name: "-a", original_path: "/a" })]}
        selectedPath={null}
        onSelect={() => {}}
        filter="all"
        onFilterChange={() => {}}
      />,
    );
    const row = screen.getByRole("option");
    // Should not throw — default browser menu appears.
    fireEvent.contextMenu(row);
  });
});
