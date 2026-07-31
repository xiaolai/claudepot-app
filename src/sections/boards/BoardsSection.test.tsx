// BoardsSection owns a polling state machine and a path that permanently
// deletes user data. Both were untested, which is a High finding in
// this project — a behavior change without a covering test.
//
// What matters here is not that the list renders. It is that:
//   - a poll never yanks the view out from under someone reading
//   - provenance is never presented as verified
//   - deletion requires a deliberate second action
//   - export refuses to write without a destination

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const boardList = vi.fn();
const boardDetail = vi.fn();
const boardDataVersion = vi.fn();
const boardRemove = vi.fn();
const boardExport = vi.fn();
const save = vi.fn();

vi.mock("../../api", () => ({
  api: {
    boardList: (...a: unknown[]) => boardList(...a),
    boardDetail: (...a: unknown[]) => boardDetail(...a),
    boardDataVersion: (...a: unknown[]) => boardDataVersion(...a),
    boardRemove: (...a: unknown[]) => boardRemove(...a),
    boardExport: (...a: unknown[]) => boardExport(...a),
  },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: (...a: unknown[]) => save(...a),
}));

import { BoardsSection } from "./BoardsSection";

const SUMMARY = {
  board_id: "b1",
  name: "nightly",
  spec_revision: 1,
  created_at: "2026-07-31T00:00:00+00:00",
  updated_at: "2026-07-31T00:00:00+00:00",
  series: ["runs"],
  total_rows: 2,
  reported_writer: "Nightly usage agent",
};

const DETAIL = {
  ...SUMMARY,
  source_board_id: null,
  series: [
    {
      name: "runs",
      columns: [{ name: "v", ty: "number" }],
      row_count: 2,
      reported_writer: "Nightly usage agent",
      last_pushed_at: "2026-07-31T00:00:00+00:00",
    },
  ],
  widgets: [],
  provenance_note: "Writers are self-declared.",
};

beforeEach(() => {
  vi.clearAllMocks();
  boardList.mockResolvedValue([SUMMARY]);
  boardDetail.mockResolvedValue(DETAIL);
  boardDataVersion.mockResolvedValue(1);
  boardRemove.mockResolvedValue(undefined);
  boardExport.mockResolvedValue("/tmp/x.json");
  save.mockResolvedValue("/tmp/x.json");
});

describe("BoardsSection", () => {
  it("renders an empty state rather than a zero-row table", async () => {
    boardList.mockResolvedValue([]);
    render(<BoardsSection />);
    expect(await screen.findByText("No boards yet.")).toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
  });

  it("labels the provenance column as reported, never as verified", async () => {
    render(<BoardsSection />);
    // rules/design.md: a status surface that shows an unverified claim
    // as fact is a High finding.
    expect(await screen.findByText("Reported writer")).toBeInTheDocument();
    expect(
      screen.getByText(/Reported by: Nightly usage agent/),
    ).toBeInTheDocument();
    expect(screen.queryByText(/^Verified/)).not.toBeInTheDocument();
  });

  it("opens a board's detail from the row control", async () => {
    render(<BoardsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "nightly" }));
    await waitFor(() => expect(boardDetail).toHaveBeenCalledWith("b1"));
  });

  it("requires a second action before deleting user data", async () => {
    render(<BoardsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "nightly" }));
    fireEvent.click(await screen.findByRole("button", { name: "Delete" }));

    // First click only arms it, and states the reason inline.
    expect(boardRemove).not.toHaveBeenCalled();
    expect(screen.getByText(/cannot be undone/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Delete permanently" }));
    await waitFor(() => expect(boardRemove).toHaveBeenCalledWith("b1"));
  });

  it("does not export when the save dialog is dismissed", async () => {
    // The command refuses a relative path, so a dismissed dialog must
    // not reach it at all.
    save.mockResolvedValue(null);
    render(<BoardsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "nightly" }));
    fireEvent.click(await screen.findByRole("button", { name: "Export" }));
    await waitFor(() => expect(save).toHaveBeenCalled());
    expect(boardExport).not.toHaveBeenCalled();
  });

  it("exports to the absolute path the user picked", async () => {
    render(<BoardsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "nightly" }));
    fireEvent.click(await screen.findByRole("button", { name: "Export" }));
    await waitFor(() =>
      expect(boardExport).toHaveBeenCalledWith("b1", "/tmp/x.json"),
    );
  });

  it("refreshes when another process commits", async () => {
    vi.useFakeTimers();
    try {
      render(<BoardsSection />);
      await vi.waitFor(() => expect(boardList).toHaveBeenCalledTimes(1));
      // First poll only establishes the baseline.
      boardDataVersion.mockResolvedValue(1);
      await vi.advanceTimersByTimeAsync(5000);
      expect(boardList).toHaveBeenCalledTimes(1);

      // Another connection committed.
      boardDataVersion.mockResolvedValue(2);
      await vi.advanceTimersByTimeAsync(5000);
      await vi.waitFor(() => expect(boardList).toHaveBeenCalledTimes(2));
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not refresh while the user is scrolling the panel", async () => {
    // The whole point of the reading guard: a poll must not replace
    // the rows someone is mid-read on.
    vi.useFakeTimers();
    try {
      const { container } = render(<BoardsSection />);
      await vi.waitFor(() => expect(boardList).toHaveBeenCalledTimes(1));

      boardDataVersion.mockResolvedValue(1);
      await vi.advanceTimersByTimeAsync(5000);

      // The section is its own scroll container now that it is not
      // nested in a tab panel.
      const root = container.querySelector("[data-board-scroll-root]");
      expect(root).not.toBeNull();
      root!.dispatchEvent(new Event("scroll"));
      boardDataVersion.mockResolvedValue(2);
      await vi.advanceTimersByTimeAsync(5000);

      // Held back, and announced instead of applied.
      expect(boardList).toHaveBeenCalledTimes(1);
      // `vi.waitFor`, not RTL's `findByText`: the latter polls on real
      // timers, which fake timers have replaced, so it hangs.
      await vi.waitFor(() =>
        expect(
          screen.getByText(/New updates arrived while you were reading/),
        ).toBeInTheDocument(),
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("refreshes the OPEN board's detail when another process commits", async () => {
    // The list refreshing is not enough: someone watching a board's
    // detail is the case boards exist for, and a poll must re-fetch
    // that detail too.
    vi.useFakeTimers();
    try {
      render(<BoardsSection />);
      // Wait for the ELEMENT, not just the call — the call resolving
      // does not mean React has committed the row yet.
      await vi.waitFor(() =>
        expect(screen.getByRole("button", { name: "nightly" })).toBeInTheDocument(),
      );
      fireEvent.click(screen.getByRole("button", { name: "nightly" }));
      await vi.waitFor(() => expect(boardDetail).toHaveBeenCalledTimes(1));

      boardDataVersion.mockResolvedValue(1);
      await vi.advanceTimersByTimeAsync(5000);
      expect(boardDetail).toHaveBeenCalledTimes(1);

      boardDataVersion.mockResolvedValue(2);
      await vi.advanceTimersByTimeAsync(5000);
      await vi.waitFor(() => expect(boardDetail).toHaveBeenCalledTimes(2));
      expect(boardDetail).toHaveBeenLastCalledWith("b1");
    } finally {
      vi.useRealTimers();
    }
  });

  it("a poll failure does not raise a banner", async () => {
    // A transient read error is not worth interrupting anyone over.
    vi.useFakeTimers();
    try {
      render(<BoardsSection />);
      await vi.waitFor(() => expect(boardList).toHaveBeenCalledTimes(1));
      boardDataVersion.mockRejectedValue(new Error("busy"));
      await vi.advanceTimersByTimeAsync(10000);
      expect(screen.queryByText(/New updates/)).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("surfaces a DETAIL load failure instead of bouncing back to the list", async () => {
    // The failure used to fall through to the list screen with no
    // explanation, because the error was only rendered inside the
    // detail view — which never mounted without a detail.
    boardDetail.mockRejectedValue(new Error("detail exploded"));
    render(<BoardsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "nightly" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      /detail exploded/,
    );
  });

  it("clears a stale detail error when a different board is opened", async () => {
    boardList.mockResolvedValue([
      SUMMARY,
      { ...SUMMARY, board_id: "b2", name: "other" },
    ]);
    boardDetail.mockRejectedValueOnce(new Error("first failed"));
    render(<BoardsSection />);

    fireEvent.click(await screen.findByRole("button", { name: "nightly" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/first failed/);

    fireEvent.click(
      screen.getByRole("button", { name: /back to the board list/i }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "other" }));
    // The second board loads fine; the first board's error must not
    // render over it.
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });

  it("ignores a stale detail response for a board the user left", async () => {
    // The dangerous shape: open A (slow), open B, A's response lands.
    // If A's detail renders under B's selection, Delete and Export read
    // `detail.board_id` and would act on the WRONG board.
    boardList.mockResolvedValue([
      SUMMARY,
      { ...SUMMARY, board_id: "b2", name: "other" },
    ]);
    let releaseFirst: (v: unknown) => void = () => {};
    boardDetail.mockImplementationOnce(
      () => new Promise((res) => (releaseFirst = res)),
    );
    boardDetail.mockImplementationOnce(async () => ({
      ...DETAIL,
      board_id: "b2",
      name: "other",
    }));

    render(<BoardsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "nightly" }));
    fireEvent.click(
      await screen.findByRole("button", { name: /back to the board list/i }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "other" }));
    await waitFor(() => expect(screen.getByText("other")).toBeInTheDocument());

    // Board A finally answers, too late.
    releaseFirst({ ...DETAIL, board_id: "b1", name: "nightly" });

    await waitFor(() =>
      expect(screen.getByText("other")).toBeInTheDocument(),
    );
    // The superseded response must not have replaced what is on screen.
    expect(screen.queryByText("nightly")).toBeNull();
  });

  it("surfaces an export failure rather than reporting success", async () => {
    boardExport.mockRejectedValue(new Error("disk full"));
    render(<BoardsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "nightly" }));
    fireEvent.click(await screen.findByRole("button", { name: "Export" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/disk full/);
  });

  it("surfaces a delete failure rather than claiming the board is gone", async () => {
    boardRemove.mockRejectedValue(new Error("locked"));
    render(<BoardsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "nightly" }));
    fireEvent.click(await screen.findByRole("button", { name: "Delete" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "Delete permanently" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(/locked/);
  });

  it("surfaces a LIST load failure instead of rendering an empty list", async () => {
    boardList.mockRejectedValue(new Error("db locked"));
    render(<BoardsSection />);
    expect(await screen.findByText(/db locked/)).toBeInTheDocument();
    expect(screen.queryByText("No boards yet.")).not.toBeInTheDocument();
  });
});
