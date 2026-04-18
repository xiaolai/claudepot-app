import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

import type { ProjectDetail as ProjectDetailData, ProjectInfo } from "../../types";

const showSpy = vi.fn();
vi.mock("../../api", () => ({
  api: {
    projectShow: (...args: unknown[]) => showSpy(...args),
  },
}));

import { ProjectDetail } from "./ProjectDetail";

function mkDetail(sessions: { id: string; size: number }[]): ProjectDetailData {
  return {
    info: {
      sanitized_name: "-p",
      original_path: "/p",
      session_count: sessions.length,
      memory_file_count: 0,
      total_size_bytes: sessions.reduce((n, s) => n + s.size, 0),
      last_modified_ms: null,
      is_orphan: false,
      is_reachable: true,
      is_empty: false,
    },
    sessions: sessions.map((s) => ({
      session_id: s.id,
      file_size: s.size,
      last_modified_ms: null,
    })),
    memory_files: [],
  };
}

const projects: ProjectInfo[] = [];

describe("ProjectDetail", () => {
  beforeEach(() => showSpy.mockReset());

  it("refetches when refreshSignal changes even if path is unchanged", async () => {
    // First response: two sessions. Second response: one session.
    showSpy
      .mockResolvedValueOnce(
        mkDetail([
          { id: "aaaa0000-0000-0000-0000-000000000000", size: 100 },
          { id: "bbbb0000-0000-0000-0000-000000000000", size: 200 },
        ]),
      )
      .mockResolvedValueOnce(
        mkDetail([
          { id: "aaaa0000-0000-0000-0000-000000000000", size: 100 },
        ]),
      );

    const { rerender } = render(
      <ProjectDetail
        path="/p"
        projects={projects}
        refreshSignal={0}
        onRename={() => {}}
        onMoved={() => {}}
      />,
    );
    await waitFor(() =>
      expect(screen.getByRole("heading", { name: /^Sessions · \d/ })).toBeInTheDocument(),
    );
    expect(
      screen.getByRole("heading", { name: /^Sessions · 2$/ }),
    ).toBeInTheDocument();

    // Simulate the parent bumping the signal after a session move.
    // path is unchanged — previous bug was that the detail pane
    // didn't refetch and kept showing the stale 2-session list.
    rerender(
      <ProjectDetail
        path="/p"
        projects={projects}
        refreshSignal={1}
        onRename={() => {}}
        onMoved={() => {}}
      />,
    );
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: /^Sessions · 1$/ }),
      ).toBeInTheDocument(),
    );
    expect(showSpy).toHaveBeenCalledTimes(2);
  });
});
