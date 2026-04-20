import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { ProjectInfo } from "../../types";
import { ProjectsTable } from "./ProjectsTable";

function mk(
  path: string,
  mods: Partial<ProjectInfo> = {},
): ProjectInfo {
  return {
    sanitized_name: path.replace(/\//g, "-"),
    original_path: path,
    session_count: 0,
    memory_file_count: 0,
    total_size_bytes: 0,
    last_modified_ms: null,
    is_orphan: false,
    is_reachable: true,
    is_empty: false,
    ...mods,
  };
}

function pathsInOrder(): string[] {
  const list = screen.getByRole("listbox", { name: "Projects" });
  const items = within(list).getAllByRole("option");
  return items.map((li) => {
    // Each row has an inner span titled with the full original_path;
    // its text content is the basename we want.
    const titled = li.querySelector<HTMLSpanElement>("span[title]");
    return titled?.textContent ?? "";
  });
}

describe("ProjectsTable sort (G10)", () => {
  const fixtures = [
    mk("/alpha", { last_modified_ms: 3_000, total_size_bytes: 100, session_count: 3 }),
    mk("/beta", { last_modified_ms: 1_000, total_size_bytes: 500, session_count: 1 }),
    mk("/gamma", { last_modified_ms: 2_000, total_size_bytes: 50, session_count: 7 }),
  ];

  it("defaults to last_touched desc — freshest first", () => {
    render(
      <ProjectsTable
        projects={fixtures}
        filter="all"
        selectedPath={null}
        onSelect={() => {}}
      />,
    );
    expect(pathsInOrder()).toEqual(["alpha", "gamma", "beta"]);
  });

  it("clicking Project column sorts asc by basename", async () => {
    const user = userEvent.setup();
    render(
      <ProjectsTable
        projects={fixtures}
        filter="all"
        selectedPath={null}
        onSelect={() => {}}
      />,
    );
    await user.click(screen.getByRole("columnheader", { name: /Project/i }));
    expect(pathsInOrder()).toEqual(["alpha", "beta", "gamma"]);
  });

  it("clicking Size twice sorts desc — biggest first", async () => {
    const user = userEvent.setup();
    render(
      <ProjectsTable
        projects={fixtures}
        filter="all"
        selectedPath={null}
        onSelect={() => {}}
      />,
    );
    const size = screen.getByRole("columnheader", { name: /Size/i });
    await user.click(size); // asc
    await user.click(size); // desc
    expect(pathsInOrder()).toEqual(["beta", "alpha", "gamma"]);
  });

  it("clicking the same column a third time restores default (last_touched desc)", async () => {
    const user = userEvent.setup();
    render(
      <ProjectsTable
        projects={fixtures}
        filter="all"
        selectedPath={null}
        onSelect={() => {}}
      />,
    );
    const sessions = screen.getByRole("columnheader", { name: /Sessions/i });
    await user.click(sessions);
    await user.click(sessions);
    await user.click(sessions);
    expect(pathsInOrder()).toEqual(["alpha", "gamma", "beta"]);
  });
});
