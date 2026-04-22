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
 * Layout-mock helpers for the virtualization tests. jsdom returns 0 for
 * every layout query, so the virtualizer collapses to zero items unless
 * we hand it a realistic scroll-container height and per-row height.
 *
 * `installLayoutStubs` returns a teardown closure that restores every
 * property descriptor it captured. Captured at install-time (not module
 * scope) so a previous test that monkey-patched the same prototype
 * surface doesn't get baked in as the "real" value.
 */
function installLayoutStubs({
  scrollHeight,
  rowHeight,
}: {
  scrollHeight: number;
  rowHeight: number;
}): () => void {
  const realRect = Object.getOwnPropertyDescriptor(
    Element.prototype,
    "getBoundingClientRect",
  );
  const realClientHeight = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    "clientHeight",
  );
  const realOffsetHeight = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    "offsetHeight",
  );

  function rectFor(el: Element): DOMRect {
    const html = el as HTMLElement;
    const isScroller = html.dataset?.testid === "sessions-table-scroll";
    const isRow = html.tagName === "LI";
    const w = 1000;
    const h = isScroller ? scrollHeight : isRow ? rowHeight : 0;
    return {
      x: 0,
      y: 0,
      width: w,
      height: h,
      top: 0,
      left: 0,
      right: w,
      bottom: h,
      toJSON() {
        return {};
      },
    } as DOMRect;
  }

  Object.defineProperty(Element.prototype, "getBoundingClientRect", {
    configurable: true,
    writable: true,
    value: function (this: Element): DOMRect {
      return rectFor(this);
    },
  });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get(): number {
      return this.dataset?.testid === "sessions-table-scroll"
        ? scrollHeight
        : 0;
    },
  });
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
    configurable: true,
    get(): number {
      if (this.dataset?.testid === "sessions-table-scroll") {
        return scrollHeight;
      }
      return this.tagName === "LI" ? rowHeight : 0;
    },
  });

  // Working ResizeObserver: the virtualizer subscribes to layout via
  // observe(); a real RO dispatches an initial entry on observe(), so
  // we mirror that synchronously. Without this the virtualizer never
  // learns the container has size and renders zero items.
  vi.stubGlobal(
    "ResizeObserver",
    class {
      private cb: ResizeObserverCallback;
      constructor(cb: ResizeObserverCallback) {
        this.cb = cb;
      }
      observe(target: Element) {
        const rect = rectFor(target);
        const entry = {
          target,
          contentRect: rect,
          borderBoxSize: [
            { inlineSize: rect.width, blockSize: rect.height },
          ] as unknown as ReadonlyArray<ResizeObserverSize>,
          contentBoxSize: [
            { inlineSize: rect.width, blockSize: rect.height },
          ] as unknown as ReadonlyArray<ResizeObserverSize>,
          devicePixelContentBoxSize: [
            { inlineSize: rect.width, blockSize: rect.height },
          ] as unknown as ReadonlyArray<ResizeObserverSize>,
        } as ResizeObserverEntry;
        this.cb([entry], this as unknown as ResizeObserver);
      }
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver,
  );

  return () => {
    if (realRect) {
      Object.defineProperty(
        Element.prototype,
        "getBoundingClientRect",
        realRect,
      );
    } else {
      delete (Element.prototype as { getBoundingClientRect?: unknown })
        .getBoundingClientRect;
    }
    if (realClientHeight) {
      Object.defineProperty(
        HTMLElement.prototype,
        "clientHeight",
        realClientHeight,
      );
    } else {
      delete (HTMLElement.prototype as { clientHeight?: unknown })
        .clientHeight;
    }
    if (realOffsetHeight) {
      Object.defineProperty(
        HTMLElement.prototype,
        "offsetHeight",
        realOffsetHeight,
      );
    } else {
      delete (HTMLElement.prototype as { offsetHeight?: unknown })
        .offsetHeight;
    }
    vi.unstubAllGlobals();
  };
}

/**
 * Above 80 rows the table flips to `@tanstack/react-virtual` so the
 * DOM stays O(viewport) instead of O(total). The stubs hand the
 * virtualizer a 600-tall container with 64px rows so it computes a
 * realistic window of ~10 visible + 8 overscan = ~18 rendered.
 */
describe("SessionsTable virtualization", () => {
  let restore: () => void;
  beforeEach(() => {
    restore = installLayoutStubs({ scrollHeight: 600, rowHeight: 64 });
  });
  afterEach(() => restore());

  it("renders only a window of rows once past the threshold", () => {
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
    // 600 / 64 ≈ 9.4 visible. Plus overscan 8 (before + after, capped
    // at the start by index 0). Tight oracle catches both "renders
    // everything" (500) and "renders nothing" (0).
    expect(rendered).toBeGreaterThanOrEqual(8);
    expect(rendered).toBeLessThanOrEqual(40);
    // Marker for the virtualized path: the listbox carries a real
    // pixel `height` (totalSize) and the rows carry data-index. Plain
    // path has neither.
    expect(list.style.height).not.toBe("");
    expect(list.querySelector("[data-index]")).not.toBeNull();
  });

  it("renders the boundary 80 rows on the plain path, 81 on the virtualized path", () => {
    const eighty: SessionRow[] = Array.from({ length: 80 }, (_, i) =>
      mk(`row${i}`),
    );
    const { unmount } = render(
      <SessionsTable
        sessions={eighty}
        filter="all"
        selectedId={null}
        onSelect={() => {}}
      />,
    );
    let list = screen.getByRole("listbox", { name: "Sessions" });
    // shown.length > THRESHOLD; 80 should NOT virtualize.
    expect(list.style.height).toBe("");
    expect(list.querySelector("[data-index]")).toBeNull();
    expect(within(list).queryAllByRole("option")).toHaveLength(80);
    unmount();

    const eightyOne: SessionRow[] = Array.from({ length: 81 }, (_, i) =>
      mk(`row${i}`),
    );
    render(
      <SessionsTable
        sessions={eightyOne}
        filter="all"
        selectedId={null}
        onSelect={() => {}}
      />,
    );
    list = screen.getByRole("listbox", { name: "Sessions" });
    // 81 SHOULD virtualize.
    expect(list.style.height).not.toBe("");
    expect(list.querySelector("[data-index]")).not.toBeNull();
  });

  it("clicking a virtualized row fires onSelect with its exact file_path", async () => {
    const user = userEvent.setup();
    const calls: string[] = [];
    // Distinct timestamps make the sort (last_active desc) deterministic:
    // shown[0] = mk(`row${499}`), shown[i] = mk(`row${499 - i}`).
    const rows: SessionRow[] = Array.from({ length: 500 }, (_, i) =>
      mk(`row${i}`, { last_modified_ms: 1_000_000 + i }),
    );
    render(
      <SessionsTable
        sessions={rows}
        filter="all"
        selectedId={null}
        onSelect={(p) => calls.push(p)}
      />,
    );
    const list = screen.getByRole("listbox", { name: "Sessions" });
    const first = within(list).getAllByRole("option")[0];
    const idx = Number(first.getAttribute("data-index"));
    expect(Number.isInteger(idx)).toBe(true);
    expect(idx).toBeGreaterThanOrEqual(0);
    await user.click(first);
    // shown[idx] under "last_active desc" with row${i}.timestamp = 1M+i
    // is row(499 - idx).
    expect(calls).toEqual([`/tmp/row${499 - idx}.jsonl`]);
  });

  it("emits aria-setsize and aria-posinset on virtualized rows", () => {
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
    const opts = within(list).getAllByRole("option");
    // Every mounted virtualized row must declare the full set size so
    // screen readers don't think there are only ~18 sessions when the
    // viewport mounts only the visible window.
    for (const opt of opts) {
      expect(opt.getAttribute("aria-setsize")).toBe("500");
      const pos = Number(opt.getAttribute("aria-posinset"));
      expect(pos).toBeGreaterThanOrEqual(1);
      expect(pos).toBeLessThanOrEqual(500);
    }
  });

  it("renders deep-search snippets verbatim on the virtualized path", () => {
    // The Rust backend redacts `sk-ant-*` substrings into the
    // `sk-ant-***<last4>` form before emitting the snippet. The UI's
    // contract is "render as text"; this guards the virtualized
    // render path against any future regression that would inline a
    // dangerouslySetInnerHTML "highlight match" wrapper and unmask
    // the token. We feed the already-redacted form (the only form
    // the wire emits) and assert it survives the round-trip without
    // a raw token reappearing.
    // Distinct timestamps + key the snippet onto the row that lands
    // first in shown[] (highest timestamp under desc sort = row499)
    // so it's guaranteed to be in the initial virtualizer window.
    const rows: SessionRow[] = Array.from({ length: 500 }, (_, i) =>
      mk(`row${i}`, { last_modified_ms: 1_000_000 + i }),
    );
    const snippets = new Map<string, string>([
      ["/tmp/row499.jsonl", "leaked sk-ant-***0000 keep searching"],
    ]);
    render(
      <SessionsTable
        sessions={rows}
        filter="all"
        selectedId={null}
        onSelect={() => {}}
        searchSnippets={snippets}
      />,
    );
    const snippet = screen.getByTestId("search-snippet");
    expect(snippet.textContent).toContain("sk-ant-***0000");
    expect(snippet.textContent ?? "").not.toMatch(/sk-ant-[A-Za-z0-9]{5,}/);
  });

  it("recovers selection across the virtualized→plain transition", () => {
    // Start above the threshold (virtualized), then narrow below it
    // (plain). Both paths must function across the rerender boundary
    // and the parent-owned selection must survive.
    const big: SessionRow[] = Array.from({ length: 200 }, (_, i) =>
      mk(`row${i}`),
    );
    const { rerender } = render(
      <SessionsTable
        sessions={big}
        filter="all"
        selectedId="/tmp/row5.jsonl"
        onSelect={() => {}}
      />,
    );
    let list = screen.getByRole("listbox", { name: "Sessions" });
    expect(list.style.height).not.toBe(""); // virtualized path
    // Narrow to 30 — drops to PlainList.
    const small = big.slice(0, 30);
    rerender(
      <SessionsTable
        sessions={small}
        filter="all"
        selectedId="/tmp/row5.jsonl"
        onSelect={() => {}}
      />,
    );
    list = screen.getByRole("listbox", { name: "Sessions" });
    expect(list.style.height).toBe(""); // plain path
    expect(within(list).queryAllByRole("option")).toHaveLength(30);
    const selected = within(list).getByRole("option", { selected: true });
    expect(selected.textContent).toContain("row5");
  });
});

/**
 * Lives outside the stubbed `describe` so it asserts the plain path
 * against the real jsdom layout (where `clientHeight` etc. are 0). If
 * the virtualization threshold ever drops below 10, this test will
 * start to fail because the virtualizer collapses to zero items
 * without the layout stubs — which is the regression we want to catch.
 */
describe("SessionsTable plain path", () => {
  it("renders every row in document flow when below the virtualization threshold", () => {
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
    // Plain-path markers: no totalSize on the <ul>, no data-index on
    // any <li>. If virtualization kicks in by accident, both flip.
    expect(list.style.height).toBe("");
    expect(list.querySelector("[data-index]")).toBeNull();
  });
});
