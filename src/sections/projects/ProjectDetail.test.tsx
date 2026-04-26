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
// Stub the app-state provider so ProjectDetail's useAppState() returns
// a minimal shape without having to mount the full provider.
vi.mock("../../providers/AppStateProvider", () => ({
  useAppState: () => ({
    status: {
      platform: "macos",
      arch: "aarch64",
      cli_active_email: null,
      desktop_active_email: null,
      desktop_installed: true,
      data_dir: "/tmp/claudepot-test",
      cc_config_dir: "/tmp/claudepot-test/.claude",
      account_count: 0,
    },
  }),
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

  it("Finder button calls revealInFinder with the fetched project's path", async () => {
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
    const btn = await screen.findByRole("button", { name: /^finder$/i });
    await user.click(btn);
    expect(revealSpy).toHaveBeenCalledWith("/p");
  });

  it("context menu exposes Reveal transcript + Copy transcript path (B4/G6)", async () => {
    showSpy.mockResolvedValue(
      mkDetail([{ id: "aaaa0000-0000-0000-0000-000000000000", size: 10 }]),
    );
    revealSpy.mockResolvedValue(undefined);
    const user = userEvent.setup();
    const writeText = vi.fn();
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });

    render(
      <ProjectDetail
        path="/p"
        projects={projects}
        refreshSignal={0}
        onRename={() => {}}
        onMoved={() => {}}
      />,
    );
    await user.click(
      await screen.findByRole("button", { name: /session actions/i }),
    );

    await user.click(screen.getByText(/reveal transcript in finder/i));
    expect(revealSpy).toHaveBeenCalledWith(
      "/tmp/claudepot-test/.claude/projects/-p/aaaa0000-0000-0000-0000-000000000000.jsonl",
    );

    // Re-open the menu and copy the path.
    await user.click(
      screen.getByRole("button", { name: /session actions/i }),
    );
    await user.click(screen.getByText(/copy transcript path/i));
    expect(writeText).toHaveBeenCalledWith(
      "/tmp/claudepot-test/.claude/projects/-p/aaaa0000-0000-0000-0000-000000000000.jsonl",
    );
  });

  it("session list paginates past 20 with Show more (B3)", async () => {
    const many = Array.from({ length: 25 }, (_, i) => ({
      id: `${String(i).padStart(4, "0")}0000-0000-0000-0000-000000000000`,
      size: 10 + i,
    }));
    showSpy.mockResolvedValue(mkDetail(many));
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
    const firstBatch = await screen.findAllByRole("option");
    expect(firstBatch).toHaveLength(20);
    expect(screen.getByText(/5 more hidden/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /show 5 more/i }));
    const after = await screen.findAllByRole("option");
    expect(after).toHaveLength(25);
  });

  it("session list filters by id prefix (B3)", async () => {
    const sessions = [
      { id: "aaaa0000-0000-0000-0000-000000000000", size: 10 },
      { id: "bbbb0000-0000-0000-0000-000000000000", size: 20 },
      { id: "cccc0000-0000-0000-0000-000000000000", size: 30 },
    ];
    showSpy.mockResolvedValue(mkDetail(sessions));
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
    await screen.findAllByRole("option");
    await user.type(screen.getByLabelText(/filter sessions/i), "bbbb");
    const filtered = await screen.findAllByRole("option");
    expect(filtered).toHaveLength(1);
  });

  it("session rows are keyboard-reachable (G16)", async () => {
    showSpy.mockResolvedValue(
      mkDetail([{ id: "aaaa0000-0000-0000-0000-000000000000", size: 10 }]),
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
    const row = await screen.findByRole("option");
    expect(row).toHaveAttribute("tabIndex", "0");
  });

  it("omits the Back button when onBack is not supplied (split-pane)", async () => {
    showSpy.mockResolvedValue(mkDetail([]));
    render(
      <ProjectDetail
        path="/p"
        projects={projects}
        refreshSignal={0}
        onRename={() => {}}
        onMoved={() => {}}
      />,
    );
    await screen.findByRole("button", { name: /^finder$/i });
    expect(
      screen.queryByRole("button", { name: /back to project list/i }),
    ).toBeNull();
  });

  it("renders a Back button that fires onBack (single-pane)", async () => {
    showSpy.mockResolvedValue(mkDetail([]));
    const onBack = vi.fn();
    const user = userEvent.setup();
    render(
      <ProjectDetail
        path="/p"
        projects={projects}
        refreshSignal={0}
        onRename={() => {}}
        onMoved={() => {}}
        onBack={onBack}
      />,
    );
    const back = await screen.findByRole("button", {
      name: /back to project list/i,
    });
    await user.click(back);
    expect(onBack).toHaveBeenCalledTimes(1);
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
    await user.click(await screen.findByRole("button", { name: /^finder$/i }));
    await waitFor(() => expect(onError).toHaveBeenCalled());
    expect(onError.mock.calls[0][0]).toMatch(/permission denied/);
  });
});
