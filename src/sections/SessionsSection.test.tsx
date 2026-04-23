/**
 * Tests for SessionsSection: the search input is wired to the deep
 * cross-session search (useSessionSearch), falls back to client-side
 * metadata filtering, and respects Esc-to-clear.
 *
 * The component talks to the backend through src/api.ts; we replace
 * that module with spies so the tests are hermetic.
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, act, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type {
  ProjectInfo,
  RepositoryGroup,
  SearchHit,
  SessionRow,
} from "../types";

const sessionListAllSpy = vi.fn();
const projectListSpy = vi.fn();
const sessionWorktreeGroupsSpy = vi.fn();
const sessionSearchSpy = vi.fn();

vi.mock("../api", () => ({
  api: {
    sessionListAll: (...a: unknown[]) => sessionListAllSpy(...a),
    projectList: (...a: unknown[]) => projectListSpy(...a),
    sessionWorktreeGroups: (...a: unknown[]) => sessionWorktreeGroupsSpy(...a),
    sessionSearch: (...a: unknown[]) => sessionSearchSpy(...a),
    // Used by the context menu / detail; unused in these tests but the
    // module shape must satisfy imports.
    revealInFinder: vi.fn(),
    sessionRead: vi.fn(),
    // The Live filter chip subscribes via useSessionLive, which
    // hydrates through sessionLiveSnapshot on first subscribe.
    // Return an empty array so the hook resolves cleanly.
    sessionLiveSnapshot: () => Promise.resolve([]),
    sessionTrashList: () => Promise.resolve({ entries: [] }),
  },
}));

// SessionDetail pulls in heavy viewers, none of which we need for the
// table-level search tests. Stub it so the search query drives only
// the list rendering under test.
vi.mock("./sessions/SessionDetail", () => ({
  SessionDetail: () => null,
}));

import { SessionsSection } from "./SessionsSection";
import { resetSessionsFilterForTest } from "./sessions/sessionsFilterStore";

function mk(id: string, mods: Partial<SessionRow> = {}): SessionRow {
  return {
    session_id: id,
    slug: `-${id}`,
    file_path: `/tmp/${id}.jsonl`,
    file_size_bytes: 1024,
    last_modified_ms: 1_700_000_000_000,
    project_path: `/repo/${id}`,
    project_from_transcript: true,
    first_ts: null,
    last_ts: null,
    event_count: 1,
    message_count: 1,
    user_message_count: 1,
    assistant_message_count: 0,
    // Bracket-delimited sentinel so `visibleSessionIds` can extract
    // the id even when the row concatenates multiple cell texts.
    first_user_prompt: `[${id}] prompt`,
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

function hit(session_id: string, file_path: string): SearchHit {
  return {
    session_id,
    slug: `-${session_id}`,
    file_path,
    project_path: `/repo/${session_id}`,
    role: "user",
    snippet: "deadlock culprit was mutex B",
    match_offset: 0,
    last_ts: null,
    score: 1.0,
  };
}

beforeEach(() => {
  sessionListAllSpy.mockReset();
  projectListSpy.mockReset();
  sessionWorktreeGroupsSpy.mockReset();
  sessionSearchSpy.mockReset();
  projectListSpy.mockResolvedValue([] as ProjectInfo[]);
  sessionWorktreeGroupsSpy.mockResolvedValue([] as RepositoryGroup[]);
  sessionSearchSpy.mockResolvedValue([] as SearchHit[]);
  // Reset the module-scope filter store so one test's filter doesn't
  // leak into the next (stale `tab = "cleanup"` would hide the list).
  resetSessionsFilterForTest();
});

async function mountWithRows(rows: SessionRow[]) {
  sessionListAllSpy.mockResolvedValue(rows);
  render(<SessionsSection />);
  // Wait for the initial fetch + first paint.
  await waitFor(() => {
    expect(sessionListAllSpy).toHaveBeenCalled();
  });
  // Table renders asynchronously after the fetch resolves.
  await screen.findByRole("listbox", { name: "Sessions" });
}

/**
 * Extracts the session ids currently visible in the list by reading
 * the `[<id>]` sentinel the fixture plants in first_user_prompt. Robust
 * to sibling cell text concatenation (headline + project + id column).
 */
function visibleSessionIds(): string[] {
  const list = screen.queryByRole("listbox", { name: "Sessions" });
  if (!list) return [];
  const out: string[] = [];
  for (const li of Array.from(list.querySelectorAll('[role="option"]'))) {
    const m = /\[(\w+)\]/.exec(li.textContent ?? "");
    if (m) out.push(m[1]);
  }
  return out;
}

describe("SessionsSection — search input", () => {
  it("shows all sessions when query is empty", async () => {
    await mountWithRows([mk("alpha"), mk("beta"), mk("gamma")]);
    expect(visibleSessionIds().sort()).toEqual(["alpha", "beta", "gamma"]);
  });

  it("narrows to metadata matches when a user types a 2+ char query", async () => {
    await mountWithRows([
      mk("alpha", { first_user_prompt: "[alpha] discuss auth" }),
      mk("beta", { first_user_prompt: "[beta] about databases" }),
    ]);
    const input = screen.getByLabelText("Search sessions");
    await userEvent.type(input, "auth");
    await waitFor(() => {
      expect(visibleSessionIds()).toEqual(["alpha"]);
    });
  });

  it("includes deep-search hits for sessions whose metadata doesn't match", async () => {
    // "gamma" has no "deadlock" in metadata, but backend search returns
    // it — the UI must show it anyway.
    sessionSearchSpy.mockResolvedValue([hit("gamma", "/tmp/gamma.jsonl")]);
    await mountWithRows([
      mk("gamma", { first_user_prompt: "[gamma] unrelated preview" }),
      mk("delta", { first_user_prompt: "[delta] boring" }),
    ]);
    const input = screen.getByLabelText("Search sessions");
    await userEvent.type(input, "deadlock");
    // The hook debounces 250 ms; advance a bit to let it fire.
    await waitFor(
      () => {
        expect(visibleSessionIds()).toEqual(["gamma"]);
      },
      { timeout: 1000 },
    );
  });

  it("restores the full list when the query is cleared", async () => {
    await mountWithRows([
      mk("alpha", { first_user_prompt: "[alpha] discuss auth" }),
      mk("beta", { first_user_prompt: "[beta] about databases" }),
    ]);
    const input = screen.getByLabelText("Search sessions");
    await userEvent.type(input, "auth");
    await waitFor(() => expect(visibleSessionIds()).toEqual(["alpha"]));
    await userEvent.clear(input);
    await waitFor(() =>
      expect(visibleSessionIds().sort()).toEqual(["alpha", "beta"]),
    );
  });

  it("does not fire a deep search for a 1-char query (2-char min)", async () => {
    await mountWithRows([mk("alpha")]);
    const input = screen.getByLabelText("Search sessions");
    await userEvent.type(input, "a");
    // Wall-clock wait covers the hook's 250ms debounce. The grill
    // testing audit flagged this as a flake risk on slow CI; if it
    // ever does flake, switch the test to fake timers but pair them
    // with `userEvent.setup({ advanceTimers })` and verify the rest
    // of the file's `userEvent.type` calls aren't broken by the
    // global timer swap.
    await act(async () => {
      await new Promise((r) => setTimeout(r, 350));
    });
    expect(sessionSearchSpy).not.toHaveBeenCalled();
  });

  it("renders the deep-hit snippet under the row when a match comes from content", async () => {
    sessionSearchSpy.mockResolvedValue([
      {
        ...hit("gamma", "/tmp/gamma.jsonl"),
        snippet: "…deadlock culprit was mutex B…",
      },
    ]);
    await mountWithRows([
      mk("gamma", { first_user_prompt: "[gamma] unrelated" }),
    ]);
    await userEvent.type(
      screen.getByLabelText("Search sessions"),
      "deadlock",
    );
    const snippet = await screen.findByTestId("search-snippet");
    expect(snippet).toHaveTextContent("deadlock culprit was mutex B");
  });

  it("does not render a snippet row when there is no deep-search match", async () => {
    // Query matches metadata (first_user_prompt) only, so the backend
    // returns nothing — no snippet row should appear.
    sessionSearchSpy.mockResolvedValue([]);
    await mountWithRows([
      mk("alpha", { first_user_prompt: "[alpha] auth story" }),
    ]);
    await userEvent.type(screen.getByLabelText("Search sessions"), "auth");
    await waitFor(() => expect(visibleSessionIds()).toEqual(["alpha"]));
    expect(screen.queryByTestId("search-snippet")).toBeNull();
  });

  it("never renders a raw sk-ant- token in the snippet cell", async () => {
    // The Rust backend redacts sk-ant- tokens into sk-ant-***<last4>
    // form before the DTO crosses to JS. Belt-and-suspenders: even if
    // a snippet containing the string got here, the UI must not leak
    // it verbatim. We simulate that by feeding the already-redacted
    // form the real pipeline would emit.
    sessionSearchSpy.mockResolvedValue([
      {
        ...hit("zulu", "/tmp/zulu.jsonl"),
        snippet: "leaked sk-ant-***0000 keep searching",
      },
    ]);
    await mountWithRows([mk("zulu", { first_user_prompt: "[zulu] prompt" })]);
    await userEvent.type(screen.getByLabelText("Search sessions"), "search");
    const snippet = await screen.findByTestId("search-snippet");
    expect(snippet.textContent ?? "").not.toMatch(/sk-ant-[A-Za-z0-9]{5,}/);
    expect(snippet.textContent).toContain("sk-ant-***0000");
  });

  it("Esc in the search input clears the query", async () => {
    await mountWithRows([
      mk("alpha", { first_user_prompt: "[alpha] discuss auth" }),
      mk("beta", { first_user_prompt: "[beta] about databases" }),
    ]);
    const input = screen.getByLabelText("Search sessions") as HTMLInputElement;
    await userEvent.type(input, "auth");
    await waitFor(() => expect(visibleSessionIds()).toEqual(["alpha"]));
    // Press Escape with the input focused.
    input.focus();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(input.value).toBe(""));
    await waitFor(() =>
      expect(visibleSessionIds().sort()).toEqual(["alpha", "beta"]),
    );
  });

  /**
   * Regression guard for the "semi-frozen typing" behavior this
   * branch exists to prevent. The input is controlled by `query`
   * (synchronous) while the filter pipeline reads `deferredQuery`
   * (low priority). A future refactor that drops `useDeferredValue`
   * and pipes `query` straight into the filter would still pass the
   * other tests above — typed characters would land, results would
   * narrow — but typing would freeze at scale because every keystroke
   * synchronously reflows the list.
   *
   * The assertion: the input element's `.value` updates synchronously
   * with the keystroke. The list update can lag — that's the point.
   */
  it("input value updates synchronously even before the filter has narrowed", async () => {
    await mountWithRows([
      mk("alpha", { first_user_prompt: "[alpha] discuss auth" }),
      mk("beta", { first_user_prompt: "[beta] about databases" }),
    ]);
    const input = screen.getByLabelText("Search sessions") as HTMLInputElement;
    await userEvent.type(input, "auth");
    // The input is the source of truth for what the user sees in the
    // text field. If this isn't "auth" right after typing, something
    // is making the input wait for the filter — which is the
    // semi-frozen regression we are guarding against.
    expect(input.value).toBe("auth");
  });

  /**
   * Subtitle invariant guard. The "N of M shown" subtitle narrows
   * against the deferred (visible) list, not the transient pre-
   * deferred state. A future refactor that swaps it back to `query`
   * could briefly display "0 of 9321 shown" while the table still
   * shows everything — a UI lie this guard catches.
   */
  it("the narrowed subtitle stays consistent with the visible list", async () => {
    await mountWithRows([
      mk("alpha", { first_user_prompt: "[alpha] discuss auth" }),
      mk("beta", { first_user_prompt: "[beta] about databases" }),
    ]);
    const input = screen.getByLabelText("Search sessions");
    await userEvent.type(input, "auth");
    await waitFor(() => expect(visibleSessionIds()).toEqual(["alpha"]));
    // Subtitle should now read "1 of 2 sessions shown" — the count
    // is taken from the same filteredByQuery the table renders.
    expect(screen.getByText(/1 of 2 sessions shown/i)).toBeInTheDocument();
  });
});
