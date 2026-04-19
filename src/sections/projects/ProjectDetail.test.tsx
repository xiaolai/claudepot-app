import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { ProjectDetail as ProjectDetailData, ProjectInfo } from "../../types";

const showSpy = vi.fn();
const revealSpy = vi.fn();
vi.mock("../../api", () => ({
  api: {
    projectShow: (...args: unknown[]) => showSpy(...args),
    revealInFinder: (...args: unknown[]) => revealSpy(...args),
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
  beforeEach(() => {
    showSpy.mockReset();
    revealSpy.mockReset();
  });

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

  it("each session row exposes a visible menu button with aria-label", async () => {
    showSpy.mockResolvedValue(
      mkDetail([
        { id: "aaaa0000-0000-0000-0000-000000000000", size: 123 },
        { id: "bbbb0000-0000-0000-0000-000000000000", size: 456 },
      ]),
    );
    render(
      <ProjectDetail
        path="/p"
        projects={projects}
        refreshSignal={0}
        onRename={() => {}}
        onMoved={() => {}}
      />,
    );
    const buttons = await screen.findAllByRole("button", {
      name: /session actions/i,
    });
    expect(buttons).toHaveLength(2);
  });

  it("menu button opens the context menu with a Move action", async () => {
    showSpy.mockResolvedValue(
      mkDetail([{ id: "aaaa0000-0000-0000-0000-000000000000", size: 123 }]),
    );
    const user = userEvent.setup();
    render(
      <ProjectDetail
        path="/p"
        projects={projects}
        refreshSignal={0}
        onRename={() => {}}
        onMoved={() => {}}
      />,
    );
    const btn = await screen.findByRole("button", { name: /session actions/i });
    await user.click(btn);
    expect(
      screen.getByText(/move to another project/i),
    ).toBeInTheDocument();
  });

  it("Open in Finder button calls revealInFinder with the fetched project's path", async () => {
    // The button uses info.original_path from the fetched ProjectDetail
    // (which may differ from the `path` prop under rename-in-flight).
    // mkDetail's fixture has original_path="/p".
    showSpy.mockResolvedValue(mkDetail([]));
    revealSpy.mockResolvedValue(undefined);
    const user = userEvent.setup();
    render(
      <ProjectDetail
        path="/p"
        projects={projects}
        refreshSignal={0}
        onRename={() => {}}
        onMoved={() => {}}
      />,
    );
    const btn = await screen.findByRole("button", { name: /open in finder/i });
    await user.click(btn);
    expect(revealSpy).toHaveBeenCalledWith("/p");
  });

  it("routes reveal errors to onError when provided", async () => {
    showSpy.mockResolvedValue(mkDetail([]));
    revealSpy.mockRejectedValue("permission denied");
    const onError = vi.fn();
    const user = userEvent.setup();
    render(
      <ProjectDetail
        path="/p"
        projects={projects}
        refreshSignal={0}
        onRename={() => {}}
        onMoved={() => {}}
        onError={onError}
      />,
    );
    await user.click(await screen.findByRole("button", { name: /open in finder/i }));
    await waitFor(() => expect(onError).toHaveBeenCalled());
    expect(onError.mock.calls[0][0]).toMatch(/permission denied/);
  });
});
