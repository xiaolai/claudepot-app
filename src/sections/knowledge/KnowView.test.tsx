/**
 * Phase 2 — the Know view.
 *
 * A project group renders memory + decision + evidence in one stream; a
 * provenance click reads the origin exchange via read_locator; cross-links
 * resolve lazily through memory_links. Plus the pure group/sort helper.
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Decision, Evidence, LessonRow } from "../../api/sharedMemory";

const lessonListSpy = vi.fn();
const listDecisionsSpy = vi.fn();
const listEvidenceSpy = vi.fn();
const readLocatorSpy = vi.fn();
const memoryLinksSpy = vi.fn();
const createMemorySpy = vi.fn();

vi.mock("../../api/sharedMemory", () => ({
  sharedMemoryApi: {
    lessonList: (...a: unknown[]) => lessonListSpy(...a),
    listDecisions: (...a: unknown[]) => listDecisionsSpy(...a),
    listEvidence: (...a: unknown[]) => listEvidenceSpy(...a),
    readLocator: (...a: unknown[]) => readLocatorSpy(...a),
    memoryLinks: (...a: unknown[]) => memoryLinksSpy(...a),
    createMemory: (...a: unknown[]) => createMemorySpy(...a),
    archiveMemory: vi.fn().mockResolvedValue(true),
    archiveDecision: vi.fn().mockResolvedValue(true),
  },
}));

import { KnowView } from "./KnowView";

const memory: LessonRow = {
  id: "m1",
  review_state: "accepted",
  kind: "constraint",
  content: "Run preflight before pushing.",
  directive: "Run scripts/preflight.sh.",
  confidence: 90,
  anchor_json: '{"files":["scripts/preflight.sh"],"evidence":"CI went red"}',
  suspect_reason: null,
  origin_file_path: "/transcripts/session-abc.jsonl",
  origin_exchange_id: "s1:4",
  compile_target: null,
  guard_ref: null,
  project_path: "/proj/app",
  created_at_ms: 3000,
};

const decision: Decision = {
  id: "d1",
  project_path: "/proj/app",
  topic: "storage",
  decision: "Use SQLite for the cache.",
  rationale: "local-first",
  status: "active",
  created_by_kind: "user",
  created_by: "user:test",
  created_at_ms: 2000,
  supersedes_id: null,
};

const evidence: Evidence = {
  id: "e1",
  project_path: "/proj/app",
  topic: "audit-fix",
  summary: "Fixed three redaction gaps.",
  verification: "cargo test green",
  files_changed_json: '["src/a.rs","src/b.rs"]',
  confidence: 88,
  created_by_kind: "agent",
  created_by: "codex@test",
  created_at_ms: 1000,
};

beforeEach(() => {
  lessonListSpy.mockResolvedValue([memory]);
  listDecisionsSpy.mockResolvedValue([decision]);
  listEvidenceSpy.mockResolvedValue([evidence]);
  readLocatorSpy.mockResolvedValue({
    file_path: memory.origin_file_path,
    exchange_id: memory.origin_exchange_id,
    line_start: 1,
    line_end: 9,
    body: "the exchange body",
    truncated: false,
  });
  memoryLinksSpy.mockResolvedValue([]);
  createMemorySpy.mockReset().mockResolvedValue({ id: "new" });
});

describe("KnowView", () => {
  it("renders memory, decision, and evidence together under the project", async () => {
    render(<KnowView onReview={vi.fn()} />);

    expect(await screen.findByText("Run preflight before pushing.")).toBeInTheDocument();
    expect(screen.getByText("Use SQLite for the cache.")).toBeInTheDocument();
    expect(screen.getByText("Fixed three redaction gaps.")).toBeInTheDocument();

    // Fetched with state="all" so the whole base surfaces at once.
    expect(lessonListSpy).toHaveBeenCalledWith(
      expect.objectContaining({ state: "all" }),
    );
  });

  it("a provenance click reads the origin exchange", async () => {
    const user = userEvent.setup();
    render(<KnowView onReview={vi.fn()} />);

    const link = await screen.findByRole("button", { name: "session-abc.jsonl" });
    await user.click(link);

    expect(readLocatorSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        file_path: "/transcripts/session-abc.jsonl",
        exchange_id: "s1:4",
      }),
    );
    expect(await screen.findByText("the exchange body")).toBeInTheDocument();
  });

  it("cross-links resolve lazily through memory_links", async () => {
    memoryLinksSpy.mockResolvedValue([
      {
        id: "l1",
        memory_id: "m1",
        decision_id: null,
        evidence_id: null,
        exchange_id: "s2:7",
        file_path: null,
        relation: "related",
      },
    ]);
    const user = userEvent.setup();
    render(<KnowView onReview={vi.fn()} />);

    const showLinks = await screen.findAllByRole("button", { name: "Show links" });
    await user.click(showLinks[0]!);

    expect(memoryLinksSpy).toHaveBeenCalledWith(
      expect.objectContaining({ memory_id: "m1" }),
    );
    expect(await screen.findByText(/related → s2:7/)).toBeInTheDocument();
  });

  it("re-review on a suspect item routes to Review", async () => {
    const onReview = vi.fn();
    lessonListSpy.mockResolvedValue([
      { ...memory, id: "m2", review_state: "suspect", suspect_reason: "code moved" },
    ]);
    listDecisionsSpy.mockResolvedValue([]);
    listEvidenceSpy.mockResolvedValue([]);
    const user = userEvent.setup();
    render(<KnowView onReview={onReview} />);

    const reReview = await screen.findByRole("button", { name: "Re-review" });
    await user.click(reReview);
    expect(onReview).toHaveBeenCalled();
  });

  it("keyboard: Enter on the focused item opens its provenance", async () => {
    listDecisionsSpy.mockResolvedValue([]);
    listEvidenceSpy.mockResolvedValue([]);
    const user = userEvent.setup();
    render(<KnowView onReview={vi.fn()} />);
    await screen.findByText("Run preflight before pushing.");

    // Cursor starts at 0 (the only item). Enter opens its excerpt.
    await user.keyboard("{Enter}");
    expect(readLocatorSpy).toHaveBeenCalled();
    expect(await screen.findByText("the exchange body")).toBeInTheDocument();
  });

  it("keyboard: j moves the cursor before Enter opens that item", async () => {
    lessonListSpy.mockResolvedValue([
      { ...memory, id: "m1", content: "first lesson", origin_file_path: "/t/first.jsonl", created_at_ms: 3000 },
      { ...memory, id: "m2", content: "second lesson", origin_file_path: "/t/second.jsonl", created_at_ms: 2000 },
    ]);
    listDecisionsSpy.mockResolvedValue([]);
    listEvidenceSpy.mockResolvedValue([]);
    const user = userEvent.setup();
    render(<KnowView onReview={vi.fn()} />);
    await screen.findByText("first lesson");

    // Sorted newest-first: m1 (3000) at cursor 0, m2 (2000) at cursor 1.
    await user.keyboard("j");
    await user.keyboard("{Enter}");
    expect(readLocatorSpy).toHaveBeenCalledWith(
      expect.objectContaining({ file_path: "/t/second.jsonl" }),
    );
  });

  it("excludes rejected lessons from the curated base", async () => {
    lessonListSpy.mockResolvedValue([
      memory,
      { ...memory, id: "m3", review_state: "rejected", content: "a rejected claim" },
    ]);
    render(<KnowView onReview={vi.fn()} />);

    expect(await screen.findByText("Run preflight before pushing.")).toBeInTheDocument();
    expect(screen.queryByText("a rejected claim")).toBeNull();
  });

  it("excludes archived decisions from the curated base", async () => {
    lessonListSpy.mockResolvedValue([]);
    listEvidenceSpy.mockResolvedValue([]);
    listDecisionsSpy.mockResolvedValue([
      decision,
      { ...decision, id: "d2", status: "archived", decision: "an archived decision" },
    ]);
    render(<KnowView onReview={vi.fn()} />);

    expect(await screen.findByText("Use SQLite for the cache.")).toBeInTheDocument();
    // Archived decisions leave the base, like archived memories do.
    expect(screen.queryByText("an archived decision")).toBeNull();
  });

  it("the project group header offers a copy-path affordance", async () => {
    listDecisionsSpy.mockResolvedValue([]);
    listEvidenceSpy.mockResolvedValue([]);
    render(<KnowView onReview={vi.fn()} />);
    await screen.findByText("Run preflight before pushing.");

    expect(
      screen.getByRole("button", { name: "Copy project path /proj/app" }),
    ).toBeInTheDocument();
  });

  it("the secondary Add affordance files a memory", async () => {
    const user = userEvent.setup();
    render(<KnowView onReview={vi.fn()} />);
    await screen.findByText("Run preflight before pushing.");

    await user.click(screen.getByRole("button", { name: "Add" }));
    await user.type(
      screen.getByRole("textbox", { name: "Memory content" }),
      "always run cargo fmt before pushing",
    );
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(createMemorySpy).toHaveBeenCalledWith(
      expect.objectContaining({ content: "always run cargo fmt before pushing" }),
    );
  });
});
