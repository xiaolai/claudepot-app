import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { SessionRow } from "../../../types";

const sessionExportToFile = vi.fn();
vi.mock("../../../api", () => ({
  api: {
    sessionExportToFile: (...args: unknown[]) => sessionExportToFile(...args),
    revealInFinder: vi.fn(),
  },
}));

vi.mock("../../../costs", () => ({
  usePriceTable: () => ({ table: null, loading: false }),
  sessionCostEstimate: () => null,
  formatUsd: (n: number) => `$${n.toFixed(2)}`,
}));

const saveDialog = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: (...args: unknown[]) => saveDialog(...args),
}));

import { SessionDetailHeader } from "./SessionDetailHeader";

function makeRow(overrides: Partial<SessionRow> = {}): SessionRow {
  return {
    session_id: "11111111-2222-3333-4444-555555555555",
    slug: "-Users-test-proj",
    file_path: "/tmp/sess.jsonl",
    file_size_bytes: 2_200_000,
    last_modified_ms: Date.now(),
    project_path: "/Users/test/proj",
    project_from_transcript: true,
    first_ts: new Date(Date.now() - 60_000).toISOString(),
    last_ts: new Date(Date.now() - 30_000).toISOString(),
    event_count: 100,
    message_count: 595,
    user_message_count: 30,
    assistant_message_count: 30,
    first_user_prompt:
      "this is a darkmode design for lixiaolai.com — read it, tell me what would you do?",
    models: ["claude-opus-4-7"],
    tokens: {
      input: 100,
      output: 200,
      cache_read: 1_000,
      cache_creation: 200,
      total: 84_000_000,
    },
    git_branch: "add-dark-mode-reborn",
    cc_version: "2.1.119",
    display_slug: null,
    has_error: false,
    is_sidechain: false,
    ...overrides,
  };
}

const callbacks = {
  onReveal: vi.fn(),
  onCopyFirstPrompt: vi.fn(),
  onMoveClick: vi.fn(),
  onToggleViewMode: vi.fn(),
  onToggleContext: vi.fn(),
};

beforeEach(() => {
  saveDialog.mockReset();
  sessionExportToFile.mockReset();
});

describe("SessionDetailHeader — full mode", () => {
  it("renders breadcrumb, title, tags, and inline Reveal + kebab", () => {
    const row = makeRow();
    render(
      <SessionDetailHeader
        row={row}
        chunks={[]}
        viewMode="chunks"
        contextOpen={false}
        compact={false}
        {...callbacks}
      />,
    );

    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent(
      /this is a darkmode design/i,
    );
    expect(
      screen.getByRole("button", { name: /reveal/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /more session actions/i }),
    ).toBeInTheDocument();

    // Secondary actions are NOT inline anymore — they live in the kebab.
    expect(
      screen.queryByRole("button", { name: /^move to project/i }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /^copy first prompt/i }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /^export$/i }),
    ).not.toBeInTheDocument();
  });

  it("kebab opens a menu with Move / Copy / Export / view toggle / context", async () => {
    const user = userEvent.setup();
    render(
      <SessionDetailHeader
        row={makeRow()}
        chunks={[]}
        viewMode="chunks"
        contextOpen={false}
        compact={false}
        {...callbacks}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /more session actions/i }),
    );

    const menu = await screen.findByRole("menu");
    expect(within(menu).getByText(/move to project/i)).toBeInTheDocument();
    expect(within(menu).getByText(/copy first prompt/i)).toBeInTheDocument();
    expect(within(menu).getByText(/export as markdown/i)).toBeInTheDocument();
    expect(within(menu).getByText(/export as json/i)).toBeInTheDocument();
    expect(within(menu).getByText(/raw events/i)).toBeInTheDocument();
    expect(within(menu).getByText(/show context/i)).toBeInTheDocument();
  });

  it("hides 'Copy first prompt' when there is no first prompt", async () => {
    const user = userEvent.setup();
    render(
      <SessionDetailHeader
        row={makeRow({ first_user_prompt: null })}
        chunks={[]}
        viewMode="chunks"
        contextOpen={false}
        compact={false}
        {...callbacks}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: /more session actions/i }),
    );
    expect(
      within(screen.getByRole("menu")).queryByText(/copy first prompt/i),
    ).not.toBeInTheDocument();
  });

  it("disables Move when project_from_transcript is false", async () => {
    const user = userEvent.setup();
    render(
      <SessionDetailHeader
        row={makeRow({ project_from_transcript: false })}
        chunks={[]}
        viewMode="chunks"
        contextOpen={false}
        compact={false}
        {...callbacks}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: /more session actions/i }),
    );
    const moveItem = within(screen.getByRole("menu")).getByRole("menuitem", {
      name: /move to project/i,
    });
    expect(moveItem).toBeDisabled();
  });

  it("omits the view-mode toggle when chunks are unavailable", async () => {
    const user = userEvent.setup();
    render(
      <SessionDetailHeader
        row={makeRow()}
        chunks={null}
        viewMode="raw"
        contextOpen={false}
        compact={false}
        {...callbacks}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: /more session actions/i }),
    );
    expect(
      within(screen.getByRole("menu")).queryByText(/raw events|chunked view/i),
    ).not.toBeInTheDocument();
  });
});

describe("SessionDetailHeader — compact mode", () => {
  it("collapses tag row + metadata row + inline buttons; keeps title, model, Reveal, kebab", () => {
    render(
      <SessionDetailHeader
        row={makeRow()}
        chunks={[]}
        viewMode="chunks"
        contextOpen={false}
        compact
        {...callbacks}
      />,
    );

    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent(
      /this is a darkmode design/i,
    );
    expect(
      screen.getByRole("button", { name: /reveal/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /more session actions/i }),
    ).toBeInTheDocument();

    // The full layout's metadata-only chips/strings (project path,
    // started/last-event timestamps, branch tag, cc version chip)
    // are dropped in compact mode.
    expect(screen.queryByText(/^Started /)).not.toBeInTheDocument();
    expect(screen.queryByText(/^Last event /)).not.toBeInTheDocument();
    expect(screen.queryByText(/cc 2\.1\.119/)).not.toBeInTheDocument();
    expect(
      screen.queryByText(/add-dark-mode-reborn/),
    ).not.toBeInTheDocument();
  });

  it("kebab still works in compact mode", async () => {
    const user = userEvent.setup();
    render(
      <SessionDetailHeader
        row={makeRow()}
        chunks={[]}
        viewMode="chunks"
        contextOpen={false}
        compact
        {...callbacks}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: /more session actions/i }),
    );
    expect(
      within(screen.getByRole("menu")).getByText(/move to project/i),
    ).toBeInTheDocument();
  });
});

describe("SessionDetailHeader — kebab export", () => {
  async function openKebab(user: ReturnType<typeof userEvent.setup>) {
    await user.click(
      screen.getByRole("button", { name: /more session actions/i }),
    );
  }

  it("invokes the export pipeline with the chosen target path (markdown)", async () => {
    const user = userEvent.setup();
    saveDialog.mockResolvedValue("/tmp/sess.md");
    sessionExportToFile.mockResolvedValue(undefined);
    render(
      <SessionDetailHeader
        row={makeRow()}
        chunks={[]}
        viewMode="chunks"
        contextOpen={false}
        compact={false}
        {...callbacks}
      />,
    );
    await openKebab(user);
    await user.click(screen.getByRole("menuitem", { name: /export as markdown/i }));

    await waitFor(() => {
      expect(saveDialog).toHaveBeenCalledTimes(1);
      expect(sessionExportToFile).toHaveBeenCalledWith(
        "/tmp/sess.jsonl",
        "md",
        "/tmp/sess.md",
      );
    });
  });

  it("invokes the export pipeline with format=json for the JSON menu item", async () => {
    const user = userEvent.setup();
    saveDialog.mockResolvedValue("/tmp/sess.json");
    sessionExportToFile.mockResolvedValue(undefined);
    const onError = vi.fn();
    render(
      <SessionDetailHeader
        row={makeRow()}
        chunks={[]}
        viewMode="chunks"
        contextOpen={false}
        compact={false}
        onError={onError}
        {...callbacks}
      />,
    );
    await openKebab(user);
    await user.click(screen.getByRole("menuitem", { name: /export as json/i }));

    await waitFor(() => {
      expect(sessionExportToFile).toHaveBeenCalledWith(
        "/tmp/sess.jsonl",
        "json",
        "/tmp/sess.json",
      );
    });
    // No error pushed on the happy path.
    expect(onError).not.toHaveBeenCalled();
  });

  it("silently bails when the user cancels the save dialog", async () => {
    const user = userEvent.setup();
    saveDialog.mockResolvedValue(null);
    const onError = vi.fn();
    render(
      <SessionDetailHeader
        row={makeRow()}
        chunks={[]}
        viewMode="chunks"
        contextOpen={false}
        compact={false}
        onError={onError}
        {...callbacks}
      />,
    );
    await openKebab(user);
    await user.click(screen.getByRole("menuitem", { name: /export as markdown/i }));

    await waitFor(() => {
      expect(saveDialog).toHaveBeenCalledTimes(1);
    });
    expect(sessionExportToFile).not.toHaveBeenCalled();
    expect(onError).not.toHaveBeenCalled();
  });

  it("redacts sk-ant-* tokens that surface in export errors", async () => {
    const user = userEvent.setup();
    saveDialog.mockResolvedValue("/tmp/sess.md");
    sessionExportToFile.mockRejectedValue(
      new Error("write failed at /tmp/sk-ant-oat01-AbcdEf12345/sess.md"),
    );
    const onError = vi.fn();
    render(
      <SessionDetailHeader
        row={makeRow()}
        chunks={[]}
        viewMode="chunks"
        contextOpen={false}
        compact={false}
        onError={onError}
        {...callbacks}
      />,
    );
    await openKebab(user);
    await user.click(screen.getByRole("menuitem", { name: /export as markdown/i }));

    await waitFor(() => {
      expect(onError).toHaveBeenCalledTimes(1);
    });
    const msg = onError.mock.calls[0][0] as string;
    expect(msg).not.toContain("AbcdEf12345");
    expect(msg).toMatch(/sk-ant-\*+/);
    expect(msg.startsWith("Export failed:")).toBe(true);
  });
});
