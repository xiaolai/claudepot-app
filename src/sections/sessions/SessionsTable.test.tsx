import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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

/**
 * Virtualization guard. Above 80 rows the table flips to
 * `@tanstack/react-virtual` so the DOM stays O(viewport) instead of
 * O(total). jsdom has no layout, so we stub `getBoundingClientRect` +
 * the Element size properties to hand the virtualizer a realistic
 * 600-tall container with ~64px rows; that lets it compute a virtual
 * window and render far fewer `<li>`s than the input array.
 */
describe("SessionsTable virtualization", () => {
  const realGBCR = Element.prototype.getBoundingClientRect;
  const realClientHeight = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    "clientHeight",
  );
  const realOffsetHeight = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    "offsetHeight",
  );

  beforeEach(() => {
    // 64px per row; 600px scroll container.
    Element.prototype.getBoundingClientRect = function (): DOMRect {
      const tag = (this as HTMLElement).tagName.toLowerCase();
      const testid = (this as HTMLElement).dataset?.testid;
      if (testid === "sessions-table-scroll") {
        return {
          x: 0,
          y: 0,
          width: 1000,
          height: 600,
          top: 0,
          left: 0,
          right: 1000,
          bottom: 600,
          toJSON() {
            return {};
          },
        } as DOMRect;
      }
      if (tag === "li") {
        return {
          x: 0,
          y: 0,
          width: 1000,
          height: 64,
          top: 0,
          left: 0,
          right: 1000,
          bottom: 64,
          toJSON() {
            return {};
          },
        } as DOMRect;
      }
      return {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        toJSON() {
          return {};
        },
      } as DOMRect;
    };
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get(): number {
        return this.dataset?.testid === "sessions-table-scroll" ? 600 : 0;
      },
    });
    Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
      configurable: true,
      get(): number {
        return this.dataset?.testid === "sessions-table-scroll"
          ? 600
          : this.tagName === "LI"
            ? 64
            : 0;
      },
    });
    // ResizeObserver is referenced by the virtualizer — jsdom doesn't
    // ship one. A no-op stub is enough because we never resize in the
    // test; the virtualizer falls back to the cached element sizes.
    (
      globalThis as unknown as { ResizeObserver: typeof ResizeObserver }
    ).ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
  });

  afterEach(() => {
    Element.prototype.getBoundingClientRect = realGBCR;
    if (realClientHeight)
      Object.defineProperty(
        HTMLElement.prototype,
        "clientHeight",
        realClientHeight,
      );
    if (realOffsetHeight)
      Object.defineProperty(
        HTMLElement.prototype,
        "offsetHeight",
        realOffsetHeight,
      );
    vi.restoreAllMocks();
  });

  it("renders fewer than the total row count once past the threshold", () => {
    const rows: SessionRow[] = Array.from({ length: 500 }, (_, i) =>
      mk(`row${i}`),
    );
    render(
      <SessionsTable
        sessions={rows}
        filter="all"
        selectedId={null}
        onSelect={() => {}}
      />,
    );
    const list = screen.getByRole("listbox", { name: "Sessions" });
    const rendered = within(list).queryAllByRole("option").length;
    // 600 / 64 ≈ 10 visible + overscan (8 before + 8 after). Anything
    // substantially smaller than the input length proves we're not
    // rendering the full list.
    expect(rendered).toBeGreaterThan(0);
    expect(rendered).toBeLessThan(rows.length / 2);
  });

  it("keeps small lists on the non-virtualized path", () => {
    const rows: SessionRow[] = Array.from({ length: 10 }, (_, i) =>
      mk(`row${i}`),
    );
    render(
      <SessionsTable
        sessions={rows}
        filter="all"
        selectedId={null}
        onSelect={() => {}}
      />,
    );
    const list = screen.getByRole("listbox", { name: "Sessions" });
    expect(within(list).queryAllByRole("option")).toHaveLength(10);
  });
});
