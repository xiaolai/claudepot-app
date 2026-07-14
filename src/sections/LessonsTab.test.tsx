/**
 * The triage inbox. Verifies the behaviors that make it a queue rather
 * than a library: accepting a lesson removes it from view and calls the
 * backend, and the gazette reports ENFORCED rather than "N stored".
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const lessonListSpy = vi.fn();
const lessonCountsSpy = vi.fn();
const lessonAcceptSpy = vi.fn();
const lessonRejectSpy = vi.fn();

vi.mock("../api/sharedMemory", () => ({
  sharedMemoryApi: {
    lessonList: (...a: unknown[]) => lessonListSpy(...a),
    lessonCounts: (...a: unknown[]) => lessonCountsSpy(...a),
    lessonAccept: (...a: unknown[]) => lessonAcceptSpy(...a),
    lessonReject: (...a: unknown[]) => lessonRejectSpy(...a),
  },
}));

import { LessonsTab } from "./LessonsTab";

const ROW = {
  id: "L1",
  review_state: "proposed" as const,
  kind: "constraint",
  content: "must call foo before bar",
  directive: "Call foo() before bar().",
  confidence: 90,
  anchor_json: JSON.stringify({ files: ["src/x.rs"], evidence: "bar panicked" }),
  suspect_reason: null,
  origin_file_path: "/t/s.jsonl",
  origin_exchange_id: null,
  project_path: "/work/app",
  created_at_ms: 1,
};

beforeEach(() => {
  lessonListSpy.mockReset().mockResolvedValue([ROW]);
  lessonAcceptSpy.mockReset().mockResolvedValue(true);
  lessonRejectSpy.mockReset().mockResolvedValue(true);
  lessonCountsSpy.mockReset().mockResolvedValue({
    proposed: 1,
    accepted: 3,
    rejected: 0,
    suspect: 0,
    enforced: 2,
  });
});

describe("LessonsTab", () => {
  it("shows the claim, its directive, and the evidence link", async () => {
    render(<LessonsTab />);
    expect(await screen.findByText("must call foo before bar")).toBeInTheDocument();
    expect(screen.getByText(/Call foo\(\) before bar\(\)/)).toBeInTheDocument();
    expect(screen.getByText(/bar panicked/)).toBeInTheDocument();
  });

  it("accepting removes the card and calls the backend", async () => {
    const user = userEvent.setup();
    render(<LessonsTab />);
    await screen.findByText("must call foo before bar");

    await user.click(screen.getByRole("button", { name: "Accept" }));

    expect(lessonAcceptSpy).toHaveBeenCalledWith({ id: "L1" });
    await waitFor(() =>
      expect(screen.queryByText("must call foo before bar")).not.toBeInTheDocument(),
    );
  });

  it("the gazette reports ENFORCED, not a vanity count", async () => {
    render(<LessonsTab />);
    // "Enforced" is a first-class stat; "N memories stored" never appears.
    expect(await screen.findByText("Enforced")).toBeInTheDocument();
    expect(screen.getByText("Documented")).toBeInTheDocument();
    // "Suspect" appears in the gazette AND the queue toggle — both legit.
    expect(screen.getAllByText("Suspect").length).toBeGreaterThan(0);
    expect(screen.queryByText(/stored/i)).not.toBeInTheDocument();
  });

  it("an empty proposed queue points at the harvest command", async () => {
    lessonListSpy.mockResolvedValue([]);
    render(<LessonsTab />);
    expect(await screen.findByText("Nothing to review.")).toBeInTheDocument();
    expect(screen.getByText(/claudepot lesson harvest/)).toBeInTheDocument();
  });
});
