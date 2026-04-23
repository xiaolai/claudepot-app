import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { AdoptReport, DiscardReport, OrphanedProject } from "../../types";

// API spies — the modal's only external surface.
const adoptSpy = vi.fn();
const discardSpy = vi.fn();
vi.mock("../../api", () => ({
  api: {
    sessionAdoptOrphan: (...args: unknown[]) => adoptSpy(...args),
    sessionDiscardOrphan: (...args: unknown[]) => discardSpy(...args),
  },
}));
// Tauri dialog plugin — returns a canned path when the user clicks Browse.
const openDialogSpy = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: unknown[]) => openDialogSpy(...args),
}));

import { AdoptOrphansModal } from "./AdoptOrphansModal";

function mkOrphan(overrides: Partial<OrphanedProject> = {}): OrphanedProject {
  return {
    slug: "-was-worktree-x",
    cwdFromTranscript: "/was/worktree/x",
    sessionCount: 2,
    totalSizeBytes: 1_500_000,
    suggestedAdoptionTarget: null,
    ...overrides,
  };
}

function mkReport(overrides: Partial<AdoptReport> = {}): AdoptReport {
  return {
    sessionsAttempted: 2,
    sessionsMoved: 2,
    sessionsFailed: [],
    sourceDirRemoved: true,
    perSession: [],
    ...overrides,
  };
}

function mkDiscardReport(
  overrides: Partial<DiscardReport> = {},
): DiscardReport {
  return {
    sessionsDiscarded: 2,
    totalSizeBytes: 1_500_000,
    dirRemoved: true,
    ...overrides,
  };
}

describe("AdoptOrphansModal", () => {
  beforeEach(() => {
    adoptSpy.mockReset();
    discardSpy.mockReset();
    openDialogSpy.mockReset();
  });

  it("renders one row per orphan with its cwd and size", () => {
    render(
      <AdoptOrphansModal
        orphans={[
          mkOrphan({ slug: "a", cwdFromTranscript: "/gone/a" }),
          mkOrphan({ slug: "b", cwdFromTranscript: "/gone/b" }),
        ]}
        onClose={() => {}}
        onCompleted={() => {}}
      />,
    );
    expect(screen.getByText("/gone/a")).toBeInTheDocument();
    expect(screen.getByText("/gone/b")).toBeInTheDocument();
  });

  it("disables Adopt until the user enters a target", async () => {
    const user = userEvent.setup();
    render(
      <AdoptOrphansModal
        orphans={[mkOrphan()]}
        onClose={() => {}}
        onCompleted={() => {}}
      />,
    );
    const adoptBtn = screen.getByRole("button", { name: /^Adopt$/ });
    expect(adoptBtn).toBeDisabled();

    const input = screen.getByPlaceholderText(/target cwd/i);
    await user.type(input, "/live/main");
    expect(adoptBtn).toBeEnabled();
  });

  it("calls the api with the typed target and shows a done row on success", async () => {
    adoptSpy.mockResolvedValue(mkReport());
    const onCompleted = vi.fn();

    const user = userEvent.setup();
    render(
      <AdoptOrphansModal
        orphans={[mkOrphan({ slug: "-dead" })]}
        onClose={() => {}}
        onCompleted={onCompleted}
      />,
    );
    await user.type(screen.getByPlaceholderText(/target cwd/i), "/live/main");
    await user.click(screen.getByRole("button", { name: /^Adopt$/ }));

    await waitFor(() => expect(adoptSpy).toHaveBeenCalledWith("-dead", "/live/main"));
    await waitFor(() =>
      expect(screen.getByText(/Adopted 2\/2 sessions/)).toBeInTheDocument(),
    );
    expect(onCompleted).toHaveBeenCalledTimes(1);
  });

  it("shows an inline error when adoption fails — no toast spam", async () => {
    adoptSpy.mockRejectedValue("sync-conflict sibling present");
    const user = userEvent.setup();
    render(
      <AdoptOrphansModal
        orphans={[mkOrphan()]}
        onClose={() => {}}
        onCompleted={() => {}}
      />,
    );
    await user.type(screen.getByPlaceholderText(/target cwd/i), "/live");
    await user.click(screen.getByRole("button", { name: /^Adopt$/ }));

    await waitFor(() =>
      expect(screen.getByText(/sync-conflict sibling present/)).toBeInTheDocument(),
    );
  });

  it("closes on Escape", async () => {
    const onClose = vi.fn();
    render(
      <AdoptOrphansModal
        orphans={[mkOrphan()]}
        onClose={onClose}
        onCompleted={() => {}}
      />,
    );
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });

  it("Remove opens a confirm dialog — no API call until the user confirms", async () => {
    const user = userEvent.setup();
    render(
      <AdoptOrphansModal
        orphans={[mkOrphan({ slug: "-dead", cwdFromTranscript: "/gone" })]}
        onClose={() => {}}
        onCompleted={() => {}}
      />,
    );
    await user.click(screen.getByRole("button", { name: /^Remove$/ }));
    // Confirm dialog heading is present.
    expect(
      screen.getByRole("heading", { name: /Move orphan to Trash\?/i }),
    ).toBeInTheDocument();
    // API NOT called yet.
    expect(discardSpy).not.toHaveBeenCalled();

    // Cancel closes the confirm without firing the API.
    await user.click(screen.getByRole("button", { name: /^Cancel$/ }));
    expect(discardSpy).not.toHaveBeenCalled();
  });

  it("confirming Remove calls sessionDiscardOrphan and shows a Trash status", async () => {
    discardSpy.mockResolvedValue(
      mkDiscardReport({ sessionsDiscarded: 2, totalSizeBytes: 1_500_000 }),
    );
    const onCompleted = vi.fn();
    const user = userEvent.setup();
    render(
      <AdoptOrphansModal
        orphans={[mkOrphan({ slug: "-dead" })]}
        onClose={() => {}}
        onCompleted={onCompleted}
      />,
    );
    await user.click(screen.getByRole("button", { name: /^Remove$/ }));
    await user.click(
      screen.getByRole("button", { name: /^Move to Trash$/ }),
    );

    await waitFor(() =>
      expect(discardSpy).toHaveBeenCalledWith("-dead"),
    );
    await waitFor(() =>
      expect(screen.getByText(/Moved 2 sessions.*Trash\./)).toBeInTheDocument(),
    );
    expect(onCompleted).toHaveBeenCalledTimes(1);
  });

  it("pre-fills target when Browse returns a picked path", async () => {
    openDialogSpy.mockResolvedValue("/picked/dir");
    const user = userEvent.setup();
    render(
      <AdoptOrphansModal
        orphans={[mkOrphan()]}
        onClose={() => {}}
        onCompleted={() => {}}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Browse/i }));
    await waitFor(() =>
      expect(screen.getByDisplayValue("/picked/dir")).toBeInTheDocument(),
    );
  });
});
