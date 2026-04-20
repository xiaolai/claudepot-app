import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { SessionRow } from "../../types";
import { SessionsTable, countSessionStatus } from "./SessionsTable";

function mk(id: string, mods: Partial<SessionRow> = {}): SessionRow {
  return {
    session_id: id,
    slug: `-${id}`,
    file_path: `/tmp/${id}.jsonl`,
    file_size_bytes: 1024,
    last_modified_ms: 1_700_000_000_000,
    project_path: "/repo/demo",
    project_from_transcript: true,
    first_ts: null,
    last_ts: null,
    event_count: 1,
    message_count: 1,
    user_message_count: 1,
    assistant_message_count: 0,
    first_user_prompt: `prompt for ${id}`,
    models: [],
    tokens: { input: 0, output: 0, cache_creation: 0, cache_read: 0, total: 0 },
    git_branch: null,
    cc_version: null,
    display_slug: null,
    has_error: false,
    is_sidechain: false,
    ...mods,
  };
}

function sessionOrder(): string[] {
  const list = screen.getByRole("listbox", { name: "Sessions" });
  return within(list)
    .getAllByRole("option")
    .map(
      (li) =>
        li
          .querySelector<HTMLSpanElement>("span[title]")
          ?.textContent?.trim() ?? "",
    );
}

describe("SessionsTable", () => {
  const fixtures = [
    mk("alpha", {
      last_ts: "2026-04-20T12:00:00Z",
      message_count: 3,
      tokens: {
        input: 100,
        output: 100,
        cache_creation: 0,
        cache_read: 0,
        total: 200,
      },
    }),
    mk("beta", {
      last_ts: "2026-04-01T00:00:00Z",
      message_count: 10,
      tokens: {
        input: 50,
        output: 50,
        cache_creation: 0,
        cache_read: 0,
        total: 100,
      },
    }),
    mk("gamma", {
      last_ts: "2026-04-10T00:00:00Z",
      message_count: 1,
      tokens: {
        input: 1,
        output: 1,
        cache_creation: 0,
        cache_read: 0,
        total: 2,
      },
    }),
  ];

  it("defaults to last_active desc — newest first", () => {
    render(
      <SessionsTable
        sessions={fixtures}
        filter="all"
        selectedId={null}
        onSelect={() => {}}
      />,
    );
    expect(sessionOrder()).toEqual([
      "prompt for alpha",
      "prompt for gamma",
      "prompt for beta",
    ]);
  });

  it("clicking Turns column sorts ascending", async () => {
    const user = userEvent.setup();
    render(
      <SessionsTable
        sessions={fixtures}
        filter="all"
        selectedId={null}
        onSelect={() => {}}
      />,
    );
    await user.click(screen.getByRole("columnheader", { name: /Turns/i }));
    expect(sessionOrder()).toEqual([
      "prompt for gamma",
      "prompt for alpha",
      "prompt for beta",
    ]);
  });

  it("errors filter shows only has_error rows", () => {
    const rows = [
      mk("ok1"),
      mk("broken", { has_error: true }),
      mk("ok2"),
    ];
    render(
      <SessionsTable
        sessions={rows}
        filter="errors"
        selectedId={null}
        onSelect={() => {}}
      />,
    );
    expect(sessionOrder()).toEqual(["prompt for broken"]);
  });

  it("agents filter shows only sidechain rows", () => {
    const rows = [mk("main"), mk("agent", { is_sidechain: true })];
    render(
      <SessionsTable
        sessions={rows}
        filter="sidechain"
        selectedId={null}
        onSelect={() => {}}
      />,
    );
    expect(sessionOrder()).toEqual(["prompt for agent"]);
  });

  it("empty list shows a ghost hint, not the table header", () => {
    render(
      <SessionsTable
        sessions={[]}
        filter="all"
        selectedId={null}
        onSelect={() => {}}
      />,
    );
    expect(screen.getByText(/No CC sessions on disk/i)).toBeInTheDocument();
    expect(
      screen.queryByRole("listbox", { name: "Sessions" }),
    ).not.toBeInTheDocument();
  });

  it("onSelect fires with file_path on row click", async () => {
    const user = userEvent.setup();
    const calls: string[] = [];
    render(
      <SessionsTable
        sessions={[mk("target")]}
        filter="all"
        selectedId={null}
        onSelect={(id) => calls.push(id)}
      />,
    );
    await user.click(screen.getByRole("option"));
    // mk() defaults file_path to `/tmp/${id}.jsonl`; the selection key
    // is file_path, not session_id, so two rows that share a session_id
    // stay unambiguous.
    expect(calls).toEqual(["/tmp/target.jsonl"]);
  });
});

describe("countSessionStatus", () => {
  it("tallies errors and sidechain independently of the total", () => {
    const rows = [
      mk("a"),
      mk("b", { has_error: true }),
      mk("c", { is_sidechain: true }),
      mk("d", { has_error: true, is_sidechain: true }),
    ];
    expect(countSessionStatus(rows)).toEqual({
      all: 4,
      errors: 2,
      sidechain: 2,
    });
  });
});
