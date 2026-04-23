import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { AbandonedCleanupReport } from "../../types";

const previewSpy = vi.fn();
const cleanupSpy = vi.fn();
vi.mock("../../api", () => ({
  api: {
    repairPreviewAbandoned: (...args: unknown[]) => previewSpy(...args),
    repairCleanupAbandoned: (...args: unknown[]) => cleanupSpy(...args),
  },
}));

import { AbandonedCleanupCard } from "./AbandonedCleanupCard";

function mkReport(
  overrides: Partial<AbandonedCleanupReport> = {},
): AbandonedCleanupReport {
  return {
    entries: [],
    removedJournals: 0,
    removedSnapshots: 0,
    bytesFreed: 0,
    ...overrides,
  };
}

function mkEntry(overrides: Partial<AbandonedCleanupReport["entries"][0]> = {}) {
  return {
    id: "move-1744800000-12345",
    journalPath: "/home/u/.claudepot/repair/journals/move-1744800000-12345.json",
    sidecarPath:
      "/home/u/.claudepot/repair/journals/move-1744800000-12345.abandoned.json",
    referencedSnapshots: [
      "/home/u/.claudepot/repair/snapshots/ts-1-P7.json",
    ],
    bytes: 2048,
    ...overrides,
  };
}

describe("AbandonedCleanupCard", () => {
  beforeEach(() => {
    previewSpy.mockReset();
    cleanupSpy.mockReset();
  });

  it("renders nothing when there are no abandoned journals (render-if-nonzero)", async () => {
    previewSpy.mockResolvedValue(mkReport({ entries: [] }));
    const { container } = render(<AbandonedCleanupCard />);
    // Wait for the probe to settle — the card may render a
    // loading-free null path, so we also assert no section is
    // emitted.
    await waitFor(() => expect(previewSpy).toHaveBeenCalled());
    await waitFor(() =>
      expect(container.querySelector(".maintenance-section")).toBeNull(),
    );
  });

  it("renders a summary when there are abandoned journals", async () => {
    previewSpy.mockResolvedValue(
      mkReport({
        entries: [
          mkEntry({ referencedSnapshots: ["/s1"], bytes: 1000 }),
          mkEntry({
            id: "move-other",
            referencedSnapshots: ["/s2", "/s3"],
            bytes: 3000,
          }),
        ],
      }),
    );

    render(<AbandonedCleanupCard />);

    // Summary line: 2 journals · 3 snapshots · <size>.
    await waitFor(() =>
      expect(screen.getByText(/2 abandoned journals/)).toBeInTheDocument(),
    );
    expect(screen.getByText(/3 snapshots/)).toBeInTheDocument();
    // Two buttons present.
    expect(screen.getByRole("button", { name: /Preview/ })).toBeEnabled();
    expect(screen.getByRole("button", { name: /^Clean$/ })).toBeEnabled();
  });

  it("opens the preview modal listing each artifact path", async () => {
    previewSpy.mockResolvedValue(
      mkReport({
        entries: [
          mkEntry({
            id: "move-abc",
            journalPath: "/j/move-abc.json",
            sidecarPath: "/j/move-abc.abandoned.json",
            referencedSnapshots: ["/s/snap-one.json"],
          }),
        ],
      }),
    );

    render(<AbandonedCleanupCard />);
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: /Preview/ }),
    );

    // Modal heading + each path visible.
    expect(
      await screen.findByRole("heading", { name: /Recovery artifacts to clean/ }),
    ).toBeInTheDocument();
    expect(screen.getByText("/j/move-abc.json")).toBeInTheDocument();
    expect(screen.getByText("/j/move-abc.abandoned.json")).toBeInTheDocument();
    expect(screen.getByText("/s/snap-one.json")).toBeInTheDocument();
  });

  it("confirms, then fires cleanup + re-probes, then calls onCleaned", async () => {
    // Two probe responses: initial list non-empty, post-clean empty.
    previewSpy
      .mockResolvedValueOnce(mkReport({ entries: [mkEntry()] }))
      .mockResolvedValueOnce(mkReport({ entries: [] }));
    cleanupSpy.mockResolvedValue(
      mkReport({ removedJournals: 1, removedSnapshots: 1, bytesFreed: 2048 }),
    );
    const onCleaned = vi.fn();

    render(<AbandonedCleanupCard onCleaned={onCleaned} />);
    const user = userEvent.setup();

    // Click Clean → opens ConfirmDialog; API NOT called yet.
    await user.click(await screen.findByRole("button", { name: /^Clean$/ }));
    expect(cleanupSpy).not.toHaveBeenCalled();

    // Confirm → cleanup runs.
    await user.click(screen.getByRole("button", { name: /^Delete$/ }));
    await waitFor(() => expect(cleanupSpy).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(onCleaned).toHaveBeenCalledTimes(1));
    // Summary disappears after the second preview returns empty.
    await waitFor(() =>
      expect(screen.queryByText(/abandoned journal/)).toBeNull(),
    );
  });

  it("Cancel on the confirm dialog leaves the card untouched", async () => {
    previewSpy.mockResolvedValue(mkReport({ entries: [mkEntry()] }));
    render(<AbandonedCleanupCard />);
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: /^Clean$/ }));
    await user.click(screen.getByRole("button", { name: /^Cancel$/ }));
    expect(cleanupSpy).not.toHaveBeenCalled();
  });
});
