import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { ProjectDetail as ProjectDetailData, ProjectInfo } from "../../types";

const showSpy = vi.fn();
const revealSpy = vi.fn();
// Cost-display dependencies. Default to a null price table and an
// empty session index so the existing list/move/reveal tests don't
// have to thread cost data — the cost trailer simply doesn't render.
// Cost-aware tests override these per-call before render().
// `Promise<unknown>` so per-test overrides can return either a real
// `PriceTableDto` / `SessionRow[]` or the default `null` / `[]`.
// `vi.fn()` so tests can `waitFor` on call-count for sync points that
// matter when asserting absence (e.g. "no cost trailer rendered").
const pricingImpl = { fn: vi.fn(() => Promise.resolve(null as unknown)) };
const sessionListImpl = {
  fn: vi.fn((..._args: unknown[]) => Promise.resolve([] as unknown[])),
};
vi.mock("../../api", () => ({
  api: {
    projectShow: (...args: unknown[]) => showSpy(...args),
    revealInFinder: (...args: unknown[]) => revealSpy(...args),
    pricingGet: () => pricingImpl.fn(),
    sessionListBySlug: (...args: unknown[]) => sessionListImpl.fn(...args),
    // ProjectDetail now mounts PermissionPanel + ProjectEnvPanel for
    // reachable projects; both fire reads on mount. Default to empty
    // so the existing list/move/reveal/cost tests don't have to
    // thread permission or .env data.
    permissionGet: (...args: unknown[]) =>
      Promise.resolve({
        projectPath: String(args[0] ?? ""),
        effectiveMode: "default",
        decidedBy: "default",
        isElevated: false,
        activeGrant: null,
      }),
    envFileList: (...args: unknown[]) =>
      Promise.resolve({ projectPath: String(args[0] ?? ""), files: [] }),
    envVaultList: () => Promise.resolve([]),
    // SessionListPane now reads useSessionLive() to render
    // per-session dots; the hook hydrates via sessionLiveSnapshot
    // on first subscribe. Return an empty list so existing tests
    // see no live sessions (which is the default state in jsdom).
    sessionLiveSnapshot: () => Promise.resolve([]),
  },
}));
// PermissionPanel subscribes to the `permission-reverted` event;
// stub the event module so the listener is a no-op in jsdom.
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
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
    pushToast: vi.fn(),
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
    // Restore cost-stub defaults — individual tests that exercise
    // the cost UI override these before render().
    pricingImpl.fn = vi.fn(() => Promise.resolve(null));
    sessionListImpl.fn = vi.fn(() => Promise.resolve([]));
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

  it("context menu exposes Reveal session + Copy session file path (B4/G6)", async () => {
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

    await user.click(screen.getByText(/reveal session in finder/i));
    expect(revealSpy).toHaveBeenCalledWith(
      "/tmp/claudepot-test/.claude/projects/-p/aaaa0000-0000-0000-0000-000000000000.jsonl",
    );

    // Re-open the menu and copy the path.
    await user.click(
      screen.getByRole("button", { name: /session actions/i }),
    );
    await user.click(screen.getByText(/copy session file path/i));
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
    // Scope to the sessions listbox — PermissionPanel's duration
    // <select> also contributes elements with the implicit `option`
    // role, which a page-wide query would wrongly count.
    const list = await screen.findByRole("listbox", { name: /sessions/i });
    const firstBatch = within(list).getAllByRole("option");
    expect(firstBatch).toHaveLength(20);
    expect(screen.getByText(/5 more hidden/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /show 5 more/i }));
    const after = await within(list).findAllByRole("option");
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
    const list = await screen.findByRole("listbox", { name: /sessions/i });
    // The filter's label widened from "filter by id prefix" to
    // "search by prompt, id, branch, or model" — id-prefix matching,
    // which this test guards, still works.
    await user.type(screen.getByLabelText(/search sessions/i), "bbbb");
    const filtered = await within(list).findAllByRole("option");
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
    const list = await screen.findByRole("listbox", { name: /sessions/i });
    const row = within(list).getByRole("option");
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

  it("renders per-session cost + total at API rates when sessions are priceable", async () => {
    // Two priceable sessions on the same model. Sonnet input price is
    // arbitrary here — the test verifies the wire-up (does the cost
    // reach the row + heading?), not the cost arithmetic, which has
    // its own tests under `src/costs.test.ts`.
    showSpy.mockResolvedValue(
      mkDetail([
        { id: "aaaa0000-0000-0000-0000-000000000000", size: 100 },
        { id: "bbbb0000-0000-0000-0000-000000000000", size: 200 },
      ]),
    );
    pricingImpl.fn = vi.fn(() =>
      Promise.resolve({
        models: {
          "claude-sonnet-4-6": {
            input_per_mtok: 3,
            output_per_mtok: 15,
            cache_write_per_mtok: 3.75,
            cache_read_per_mtok: 0.3,
          },
        },
        source: { kind: "bundled", timestamp: "2026-01-01", url: "" },
        last_fetch_error: null,
      }),
    );
    sessionListImpl.fn = vi.fn(() =>
      Promise.resolve([
        {
          session_id: "aaaa0000-0000-0000-0000-000000000000",
          slug: "-p",
          file_path: "/tmp/claudepot-test/.claude/projects/-p/a.jsonl",
          file_size_bytes: 100,
          last_modified_ms: null,
          project_path: "/p",
          project_from_transcript: true,
          first_ts: null,
          last_ts: null,
          event_count: 0,
          message_count: 0,
          user_message_count: 0,
          assistant_message_count: 0,
          first_user_prompt: null,
          models: ["claude-sonnet-4-6"],
          tokens: {
            input: 1_000_000,
            output: 200_000,
            cache_creation: 0,
            cache_read: 0,
            total: 1_200_000,
          },
          git_branch: null,
          cc_version: null,
          display_slug: null,
          has_error: false,
          is_sidechain: false,
        },
        {
          session_id: "bbbb0000-0000-0000-0000-000000000000",
          slug: "-p",
          file_path: "/tmp/claudepot-test/.claude/projects/-p/b.jsonl",
          file_size_bytes: 200,
          last_modified_ms: null,
          project_path: "/p",
          project_from_transcript: true,
          first_ts: null,
          last_ts: null,
          event_count: 0,
          message_count: 0,
          user_message_count: 0,
          assistant_message_count: 0,
          first_user_prompt: null,
          models: ["claude-sonnet-4-6"],
          tokens: {
            input: 500_000,
            output: 100_000,
            cache_creation: 0,
            cache_read: 0,
            total: 600_000,
          },
          git_branch: null,
          cc_version: null,
          display_slug: null,
          has_error: false,
          is_sidechain: false,
        },
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
    // Heading shows the project total — 1M·$3 + 200k·$15 + 500k·$3 +
    // 100k·$15 = 3 + 3 + 1.5 + 1.5 = $9.00, which `formatUsd` renders
    // as "$9.00" (>= $0.01 path).
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: /Sessions · 2 · \$9\.00/ }),
      ).toBeInTheDocument(),
    );
    // Per-row cost rendered as $6.00 and $3.00 respectively. Match on
    // the dollar value rather than the row container so the test
    // doesn't lock in the surrounding `· · ·` formatting.
    expect(screen.getByText(/\$6\.00/)).toBeInTheDocument();
    expect(screen.getByText(/\$3\.00/)).toBeInTheDocument();
  });

  it("suppresses the cost trailer when no session matches the price table", async () => {
    showSpy.mockResolvedValue(
      mkDetail([{ id: "aaaa0000-0000-0000-0000-000000000000", size: 100 }]),
    );
    // Price table covers a different model — the session uses a model
    // that doesn't resolve, so `sessionCostEstimate` returns null and
    // the UI must render NO cost trailer (not "$0.00").
    pricingImpl.fn = vi.fn(() =>
      Promise.resolve({
        models: {
          "claude-opus-4-7": {
            input_per_mtok: 15,
            output_per_mtok: 75,
            cache_write_per_mtok: 18.75,
            cache_read_per_mtok: 1.5,
          },
        },
        source: { kind: "bundled", timestamp: "2026-01-01", url: "" },
        last_fetch_error: null,
      }),
    );
    sessionListImpl.fn = vi.fn(() =>
      Promise.resolve([
        {
          session_id: "aaaa0000-0000-0000-0000-000000000000",
          slug: "-p",
          file_path: "/tmp/claudepot-test/.claude/projects/-p/a.jsonl",
          file_size_bytes: 100,
          last_modified_ms: null,
          project_path: "/p",
          project_from_transcript: true,
          first_ts: null,
          last_ts: null,
          event_count: 0,
          message_count: 0,
          user_message_count: 0,
          assistant_message_count: 0,
          first_user_prompt: null,
          models: ["claude-sonnet-4-6"],
          tokens: {
            input: 1_000_000,
            output: 200_000,
            cache_creation: 0,
            cache_read: 0,
            total: 1_200_000,
          },
          git_branch: null,
          cc_version: null,
          display_slug: null,
          has_error: false,
          is_sidechain: false,
        },
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
    // The cost pipeline calls BOTH endpoints — wait until each has
    // been invoked so the test can't pass before the cost effect ran.
    // Otherwise an early `queryByText(/\$/)` would tautologically pass
    // even if suppression were broken.
    // The per-project listing must request only this project's slug —
    // shipping the whole cross-project index per click is the shape
    // `session_list_by_slug` exists to avoid.
    await waitFor(() => expect(sessionListImpl.fn).toHaveBeenCalledWith("-p"));
    await waitFor(() => expect(pricingImpl.fn).toHaveBeenCalled());
    // Flush one more microtask cycle so the useMemo + setState that
    // would emit a trailer (if one were going to) has a chance to run.
    await Promise.resolve();
    await Promise.resolve();
    // The heading must NOT carry the cost trailer for unpriced sessions.
    expect(
      screen.getByRole("heading", { name: /^Sessions · 1$/ }),
    ).toBeInTheDocument();
    expect(screen.queryByText(/\$/)).not.toBeInTheDocument();
  });

  // ─── session search ────────────────────────────────────────────
  //
  // The list used to render 8 hex characters of a UUID per row and
  // filter on `session_id` only — nothing to scan, and findable only
  // if you already knew the id. Rows now lead with the first user
  // prompt and the filter matches prompt / id / branch / model.

  /** A SessionRow as `sessionListBySlug` returns it. */
  function mkRow(
    id: string,
    prompt: string | null,
    branch: string | null = null,
  ) {
    return {
      session_id: id,
      slug: "-p",
      file_path: `/tmp/claudepot-test/.claude/projects/-p/${id}.jsonl`,
      file_size_bytes: 100,
      last_modified_ms: null,
      project_path: "/p",
      project_from_transcript: true,
      first_ts: null,
      last_ts: null,
      event_count: 0,
      message_count: 0,
      user_message_count: 0,
      assistant_message_count: 0,
      first_user_prompt: prompt,
      models: ["claude-sonnet-4-6"],
      tokens: {
        input: 0,
        output: 0,
        cache_creation: 0,
        cache_read: 0,
        total: 0,
      },
      git_branch: branch,
      cc_version: null,
      display_slug: null,
      has_error: false,
      is_sidechain: false,
    };
  }

  const ID_A = "aaaa0000-0000-0000-0000-000000000000";
  const ID_B = "bbbb0000-0000-0000-0000-000000000000";

  async function renderWithRows(rows: unknown[]) {
    showSpy.mockResolvedValue(
      mkDetail([
        { id: ID_A, size: 100 },
        { id: ID_B, size: 200 },
      ]),
    );
    sessionListImpl.fn = vi.fn(() => Promise.resolve(rows));
    render(
      <ProjectDetail
        path="/p"
        projects={projects}
        refreshSignal={0}
        onRename={() => {}}
        onMoved={() => {}}
      />,
    );
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: /^Sessions · 2$/ }),
      ).toBeInTheDocument(),
    );
    await waitFor(() => expect(sessionListImpl.fn).toHaveBeenCalled());
  }

  const searchBox = () =>
    screen.getByRole("searchbox", { name: /search sessions/i });

  it("labels session rows with the first user prompt, not the bare uuid", async () => {
    await renderWithRows([
      mkRow(ID_A, "investigate the deadlock in the mutex"),
      mkRow(ID_B, "write the release notes"),
    ]);
    await waitFor(() =>
      expect(
        screen.getByText(/investigate the deadlock in the mutex/),
      ).toBeInTheDocument(),
    );
    expect(screen.getByText(/write the release notes/)).toBeInTheDocument();
    // The id is still reachable — demoted to the meta line, not dropped.
    expect(screen.getByText(ID_A.slice(0, 8))).toBeInTheDocument();
  });

  it("filters sessions by prompt text", async () => {
    await renderWithRows([
      mkRow(ID_A, "investigate the deadlock in the mutex"),
      mkRow(ID_B, "write the release notes"),
    ]);
    await waitFor(() =>
      expect(screen.getByText(/investigate the deadlock/)).toBeInTheDocument(),
    );

    await userEvent.type(searchBox(), "deadlock");

    expect(screen.getByText(/investigate the deadlock/)).toBeInTheDocument();
    expect(screen.queryByText(/write the release notes/)).toBeNull();
  });

  it("filters sessions by git branch", async () => {
    await renderWithRows([
      mkRow(ID_A, "first task", "feat/rotation"),
      mkRow(ID_B, "second task", "main"),
    ]);
    await waitFor(() =>
      expect(screen.getByText(/first task/)).toBeInTheDocument(),
    );

    await userEvent.type(searchBox(), "rotation");

    expect(screen.getByText(/first task/)).toBeInTheDocument();
    expect(screen.queryByText(/second task/)).toBeNull();
  });

  it("still filters by session-id prefix", async () => {
    await renderWithRows([
      mkRow(ID_A, "investigate the deadlock"),
      mkRow(ID_B, "write the release notes"),
    ]);
    await waitFor(() =>
      expect(screen.getByText(/investigate the deadlock/)).toBeInTheDocument(),
    );

    await userEvent.type(searchBox(), "bbbb");

    expect(screen.getByText(/write the release notes/)).toBeInTheDocument();
    expect(screen.queryByText(/investigate the deadlock/)).toBeNull();
  });

  it("falls back to id label and id search when the index has no rows", async () => {
    // Cold index (or fetch in flight): no prompts available. The row
    // must still render and still be findable by id — search never
    // goes fully dark.
    await renderWithRows([]);

    expect(screen.getByText(ID_A.slice(0, 8))).toBeInTheDocument();

    await userEvent.type(searchBox(), "bbbb");

    expect(screen.getByText(ID_B.slice(0, 8))).toBeInTheDocument();
    expect(screen.queryByText(ID_A.slice(0, 8))).toBeNull();
  });
});
