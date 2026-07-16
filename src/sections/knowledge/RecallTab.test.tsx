/**
 * Recall — transcript full-text search.
 *
 * Verifies the search/paging contract and the guards the rewrite added: a
 * blank query never reaches the backend, and "Load more" appends the next
 * page rather than replacing the current one.
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { SearchHit } from "../../api/sharedMemory";

const searchSpy = vi.fn();
const readLocatorSpy = vi.fn();

vi.mock("../../api/sharedMemory", () => ({
  sharedMemoryApi: {
    search: (...a: unknown[]) => searchSpy(...a),
    readLocator: (...a: unknown[]) => readLocatorSpy(...a),
  },
}));

import { RecallTab } from "./RecallTab";

const hit = (id: string): SearchHit => ({
  exchange_id: id,
  file_path: `/t/${id}.jsonl`,
  session_id: "s",
  source_kind: "claude_code",
  project_path: "/proj/app",
  git_branch: null,
  timestamp_ms: null,
  line_start: null,
  line_end: null,
  snippet: `snippet ${id}`,
  turn_index: 0,
});

beforeEach(() => {
  searchSpy.mockReset().mockResolvedValue({ hits: [hit("a")], has_more: false });
  readLocatorSpy.mockReset().mockResolvedValue({
    file_path: "/t/a.jsonl",
    exchange_id: "a",
    line_start: 1,
    line_end: 2,
    body: "body a",
    truncated: false,
  });
});

describe("RecallTab", () => {
  it("searching renders hits and pages from offset 0", async () => {
    const user = userEvent.setup();
    render(<RecallTab />);
    await user.type(screen.getByRole("textbox", { name: "Search query" }), "rate limiter");
    await user.click(screen.getByRole("button", { name: "Search" }));

    expect(await screen.findByText("snippet a")).toBeInTheDocument();
    expect(searchSpy).toHaveBeenCalledWith(
      expect.objectContaining({ query: "rate limiter", offset: 0 }),
    );
  });

  it("a whitespace query submitted via Enter never reaches the backend", async () => {
    const user = userEvent.setup();
    render(<RecallTab />);
    const input = screen.getByRole("textbox", { name: "Search query" });
    await user.type(input, "   {Enter}");
    expect(searchSpy).not.toHaveBeenCalled();
  });

  it("Load more appends the next page instead of replacing it", async () => {
    searchSpy
      .mockResolvedValueOnce({ hits: [hit("a")], has_more: true })
      .mockResolvedValueOnce({ hits: [hit("b")], has_more: false });
    const user = userEvent.setup();
    render(<RecallTab />);
    await user.type(screen.getByRole("textbox", { name: "Search query" }), "q");
    await user.click(screen.getByRole("button", { name: "Search" }));
    await screen.findByText("snippet a");

    await user.click(screen.getByRole("button", { name: "Load more" }));

    expect(await screen.findByText("snippet b")).toBeInTheDocument();
    expect(screen.getByText("snippet a")).toBeInTheDocument();
    expect(searchSpy).toHaveBeenLastCalledWith(
      expect.objectContaining({ offset: 1 }),
    );
  });

  it("reading an excerpt loads it under the hit", async () => {
    const user = userEvent.setup();
    render(<RecallTab />);
    await user.type(screen.getByRole("textbox", { name: "Search query" }), "q");
    await user.click(screen.getByRole("button", { name: "Search" }));
    await screen.findByText("snippet a");

    await user.click(screen.getByRole("button", { name: "Read excerpt" }));
    expect(await screen.findByText("body a")).toBeInTheDocument();
  });
});
