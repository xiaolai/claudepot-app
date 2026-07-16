/**
 * Phase 3 — pending recurrences in Review.
 *
 * Unconfirmed candidates render here (never in the dashboard count); a
 * human's Confirm turns the soft signal into a counted, actionable datum.
 * The panel stays invisible when there is nothing pending.
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const recurrenceListSpy = vi.fn();
const recurrenceConfirmSpy = vi.fn();
const recurrenceDismissSpy = vi.fn();

vi.mock("../../api/sharedMemory", () => ({
  sharedMemoryApi: {
    recurrenceList: (...a: unknown[]) => recurrenceListSpy(...a),
    recurrenceConfirm: (...a: unknown[]) => recurrenceConfirmSpy(...a),
    recurrenceDismiss: (...a: unknown[]) => recurrenceDismissSpy(...a),
  },
}));

import { RecurrencePanel } from "./RecurrencePanel";

const EVENT = {
  id: "r1",
  matched_memory_id: "m1",
  project_path: "/proj/app",
  new_content: "foo must be initialised first",
  new_exchange_id: "s2:3",
  new_file_path: "/transcripts/new.jsonl",
  detected_by: "anchor" as const,
  detected_at_ms: 100,
  status: "pending" as const,
  confirmed_at_ms: null,
  matched_content: "call foo before bar",
  matched_state: "accepted",
};

beforeEach(() => {
  recurrenceListSpy.mockReset().mockResolvedValue([]);
  recurrenceConfirmSpy.mockReset().mockResolvedValue(true);
  recurrenceDismissSpy.mockReset().mockResolvedValue(true);
});

describe("RecurrencePanel", () => {
  it("renders nothing when there are no pending recurrences", async () => {
    const { container } = render(<RecurrencePanel />);
    // Let the effect settle; still empty.
    await Promise.resolve();
    expect(container.querySelector("section")).toBeNull();
  });

  it("shows the recurring claim and what was already learned", async () => {
    recurrenceListSpy.mockResolvedValue([EVENT]);
    render(<RecurrencePanel />);

    expect(
      await screen.findByText("foo must be initialised first"),
    ).toBeInTheDocument();
    expect(screen.getByText(/already learned: call foo before bar/)).toBeInTheDocument();
  });

  it("Confirm calls the backend and drops the row", async () => {
    recurrenceListSpy.mockResolvedValue([EVENT]);
    const user = userEvent.setup();
    render(<RecurrencePanel />);

    const confirm = await screen.findByRole("button", { name: /Confirm/ });
    await user.click(confirm);

    expect(recurrenceConfirmSpy).toHaveBeenCalledWith("r1");
    expect(screen.queryByText("foo must be initialised first")).toBeNull();
  });

  it("Dismiss calls the backend and drops the row", async () => {
    recurrenceListSpy.mockResolvedValue([EVENT]);
    const user = userEvent.setup();
    render(<RecurrencePanel />);

    const dismiss = await screen.findByRole("button", { name: "Dismiss" });
    await user.click(dismiss);

    expect(recurrenceDismissSpy).toHaveBeenCalledWith("r1");
    expect(screen.queryByText("foo must be initialised first")).toBeNull();
  });

  it("can deep-link the matched lesson into Know", async () => {
    recurrenceListSpy.mockResolvedValue([EVENT]);
    const onOpenMemory = vi.fn();
    const user = userEvent.setup();
    render(<RecurrencePanel onOpenMemory={onOpenMemory} />);

    await user.click(await screen.findByRole("button", { name: "Open in Know" }));

    expect(onOpenMemory).toHaveBeenCalledWith("/proj/app", "m1");
  });
});
