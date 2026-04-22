import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { TrashListing } from "../../types";

const trashListSpy = vi.fn();
const trashRestoreSpy = vi.fn();
const trashEmptySpy = vi.fn();

vi.mock("../../api", () => ({
  api: {
    sessionTrashList: (...a: unknown[]) => trashListSpy(...a),
    sessionTrashRestore: (...a: unknown[]) => trashRestoreSpy(...a),
    sessionTrashEmpty: (...a: unknown[]) => trashEmptySpy(...a),
  },
}));

import { TrashDrawer } from "./TrashDrawer";

function mkListing(entries: number): TrashListing {
  return {
    entries: Array.from({ length: entries }, (_, i) => ({
      id: `batch-${i}`,
      kind: i % 2 === 0 ? ("prune" as const) : ("slim" as const),
      orig_path: `/tmp/s${i}.jsonl`,
      size: (i + 1) * 1000,
      ts_ms: 1_700_000_000_000,
      cwd: null,
      reason: null,
    })),
    total_bytes: entries * 1000,
  };
}

beforeEach(() => {
  trashListSpy.mockReset();
  trashRestoreSpy.mockReset();
  trashEmptySpy.mockReset();
});

describe("TrashDrawer", () => {
  it("renders an empty-state message when the trash is empty", async () => {
    trashListSpy.mockResolvedValue(mkListing(0));
    render(<TrashDrawer />);
    await waitFor(() => {
      expect(screen.getByText(/Trash is empty/)).toBeInTheDocument();
    });
  });

  it("lists each entry and calls restore with its id", async () => {
    trashListSpy.mockResolvedValue(mkListing(2));
    trashRestoreSpy.mockResolvedValue("/tmp/s0.jsonl");
    render(<TrashDrawer />);
    const entries = await screen.findAllByTestId("trash-entry");
    expect(entries).toHaveLength(2);
    // Second call returns an empty list so the drawer re-renders without crashing.
    trashListSpy.mockResolvedValueOnce(mkListing(1));
    await userEvent.click(
      screen.getAllByRole("button", { name: /Restore/i })[0],
    );
    await waitFor(() => {
      expect(trashRestoreSpy).toHaveBeenCalledWith("batch-0");
    });
  });

  it("requires explicit confirmation before calling sessionTrashEmpty", async () => {
    trashListSpy.mockResolvedValue(mkListing(1));
    trashEmptySpy.mockResolvedValue(500);
    render(<TrashDrawer />);
    await screen.findAllByTestId("trash-entry");
    await userEvent.click(
      screen.getByRole("button", { name: /Empty trash/ }),
    );
    // Confirm button now visible.
    trashListSpy.mockResolvedValueOnce(mkListing(0));
    await userEvent.click(screen.getByTestId("confirm-empty"));
    await waitFor(() => {
      expect(trashEmptySpy).toHaveBeenCalledTimes(1);
    });
  });
});
