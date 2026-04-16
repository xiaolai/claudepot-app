import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
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
      mk({
        sanitized_name: "-live",
        original_path: "/live",
      }),
      mk({
        sanitized_name: "-gone",
        original_path: "/deleted",
        is_orphan: true,
        is_reachable: true,
      }),
      mk({
        sanitized_name: "-unmounted",
        original_path: "/Volumes/ext/proj",
        is_orphan: false,
        is_reachable: false,
      }),
      mk({
        sanitized_name: "-abandoned",
        original_path: "/tmp/abandoned",
        is_orphan: true,
        is_empty: true,
        session_count: 0,
      }),
    ];

    render(
      <ProjectsList
        projects={projects}
        selectedPath={null}
        onSelect={() => {}}
        filter="all"
        onFilterChange={() => {}}
        onClean={() => {}}
        cleanCount={2}
      />,
    );

    // Aria labels distinguish the three non-alive states.
    expect(
      screen.getByLabelText(/orphan — source dir missing/),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText(/unreachable — mount the source volume/),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText(/empty — CC project dir has no content/),
    ).toBeInTheDocument();
  });

  it("filter chips report accurate counts", () => {
    const projects: ProjectInfo[] = [
      mk({ sanitized_name: "a", original_path: "/a" }),
      mk({
        sanitized_name: "b",
        original_path: "/b",
        is_orphan: true,
      }),
      mk({
        sanitized_name: "c",
        original_path: "/c",
        is_orphan: true,
      }),
      mk({
        sanitized_name: "d",
        original_path: "/Volumes/ext/d",
        is_reachable: false,
      }),
    ];

    render(
      <ProjectsList
        projects={projects}
        selectedPath={null}
        onSelect={() => {}}
        filter="all"
        onFilterChange={() => {}}
        onClean={() => {}}
        cleanCount={2}
      />,
    );

    const orphanChip = screen.getByRole("tab", { name: /Orphan/ });
    // Chip text is "Orphan 2" — verify the count is present.
    expect(orphanChip.textContent).toMatch(/2/);

    const unreachableChip = screen.getByRole("tab", { name: /Unreachable/ });
    expect(unreachableChip.textContent).toMatch(/1/);

    const emptyChip = screen.getByRole("tab", { name: /Empty/ });
    expect(emptyChip.textContent).toMatch(/0/);
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
        onClean={() => {}}
        cleanCount={1}
      />,
    );
    await user.click(screen.getByRole("tab", { name: /Orphan/ }));
    expect(spy).toHaveBeenCalledWith("orphan");
  });

  it("Clean button is disabled when cleanCount=0 and shows count otherwise", async () => {
    const user = userEvent.setup();
    const spy = vi.fn();

    const { rerender } = render(
      <ProjectsList
        projects={[mk({})]}
        selectedPath={null}
        onSelect={() => {}}
        filter="all"
        onFilterChange={() => {}}
        onClean={spy}
        cleanCount={0}
      />,
    );
    const disabledBtn = screen.getByRole("button", { name: /Clean…/ });
    expect(disabledBtn).toBeDisabled();

    rerender(
      <ProjectsList
        projects={[mk({ is_orphan: true })]}
        selectedPath={null}
        onSelect={() => {}}
        filter="all"
        onFilterChange={() => {}}
        onClean={spy}
        cleanCount={3}
      />,
    );
    const enabledBtn = screen.getByRole("button", { name: /Clean \(3\)…/ });
    expect(enabledBtn).toBeEnabled();
    await user.click(enabledBtn);
    expect(spy).toHaveBeenCalledTimes(1);
  });
});
