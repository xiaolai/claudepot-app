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
const logDecisionSpy = vi.fn();

vi.mock("../../api/sharedMemory", () => ({
  sharedMemoryApi: {
    lessonList: (...a: unknown[]) => lessonListSpy(...a),
    listDecisions: (...a: unknown[]) => listDecisionsSpy(...a),
    listEvidence: (...a: unknown[]) => listEvidenceSpy(...a),
    readLocator: (...a: unknown[]) => readLocatorSpy(...a),
    memoryLinks: (...a: unknown[]) => memoryLinksSpy(...a),
    createMemory: (...a: unknown[]) => createMemorySpy(...a),
    logDecision: (...a: unknown[]) => logDecisionSpy(...a),
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
  logDecisionSpy.mockReset().mockResolvedValue({ id: "new-decision" });
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
    // Routes to the *suspect* queue, not the default proposed one.
    expect(onReview).toHaveBeenCalledWith("suspect");
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
    await user.click(screen.getByRole("button", { name: "Save memory" }));

    expect(createMemorySpy).toHaveBeenCalledWith(
      expect.objectContaining({ content: "always run cargo fmt before pushing" }),
    );
  });

  it("the same secondary Add affordance can log a decision", async () => {
    const user = userEvent.setup();
    render(<KnowView onReview={vi.fn()} />);
    await screen.findByText("Run preflight before pushing.");

    await user.click(screen.getByRole("button", { name: "Add" }));
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Knowledge type" }),
      "decision",
    );
    await user.type(
      screen.getByRole("textbox", { name: "Decision" }),
      "Keep the cache local",
    );
    await user.type(
      screen.getByRole("textbox", { name: "Decision rationale" }),
      "It avoids network coupling",
    );
    await user.click(screen.getByRole("button", { name: "Save decision" }));

    expect(logDecisionSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        decision: "Keep the cache local",
        rationale: "It avoids network coupling",
      }),
    );
  });

  it("a deep-link to an uncurated project names it and offers a way out", async () => {
    // The project has nothing curated, so it isn't in any loaded item — the
    // exact case the Dashboard's 'busy-unmined' rows deep-link into.
    lessonListSpy.mockResolvedValue([]);
    listDecisionsSpy.mockResolvedValue([]);
    listEvidenceSpy.mockResolvedValue([]);
    const user = userEvent.setup();
    render(<KnowView initialProjectFilter="/proj/empty" onReview={vi.fn()} />);

    // The empty state names the project, not a generic "nothing matches".
    expect(await screen.findByText(/Nothing curated in/)).toBeInTheDocument();
    // The project select shows the injected value, never silently "All".
    const projectSelect = screen.getByRole("combobox", { name: "Project filter" });
    expect(projectSelect).toHaveValue("/proj/empty");
    // And there is an explicit way back to the whole base.
    await user.click(screen.getByRole("button", { name: "Clear project filter" }));
    expect(projectSelect).toHaveValue("all");
  });

  it("the Accepted filter includes enforced lessons", async () => {
    lessonListSpy.mockResolvedValue([
      { ...memory, id: "a1", review_state: "accepted", compile_target: null, content: "plain accepted" },
      {
        ...memory,
        id: "a2",
        review_state: "accepted",
        compile_target: "guard",
        guard_ref: "scripts/repo-invariants.sh:1",
        content: "enforced one",
      },
    ]);
    listDecisionsSpy.mockResolvedValue([]);
    listEvidenceSpy.mockResolvedValue([]);
    const user = userEvent.setup();
    render(<KnowView onReview={vi.fn()} />);
    await screen.findByText("plain accepted");

    await user.selectOptions(
      screen.getByRole("combobox", { name: "State filter" }),
      "accepted",
    );
    // "Accepted" is the superset — it must not hide the enforced item.
    expect(screen.getByText("plain accepted")).toBeInTheDocument();
    expect(screen.getByText("enforced one")).toBeInTheDocument();
  });

  it("an accepted, un-enforced memory offers the compile command", async () => {
    lessonListSpy.mockResolvedValue([
      { ...memory, review_state: "accepted", compile_target: null },
    ]);
    listDecisionsSpy.mockResolvedValue([]);
    listEvidenceSpy.mockResolvedValue([]);
    render(<KnowView onReview={vi.fn()} />);
    await screen.findByText("Run preflight before pushing.");

    expect(
      screen.getByRole("button", { name: "Copy compile command" }),
    ).toBeInTheDocument();
  });

  it("the free-text filter narrows the base by the record's own words", async () => {
    lessonListSpy.mockResolvedValue([
      { ...memory, id: "m1", content: "run preflight" },
      { ...memory, id: "m2", content: "use sqlite cache" },
    ]);
    listDecisionsSpy.mockResolvedValue([]);
    listEvidenceSpy.mockResolvedValue([]);
    const user = userEvent.setup();
    render(<KnowView onReview={vi.fn()} />);
    await screen.findByText("run preflight");

    await user.type(
      screen.getByRole("textbox", { name: "Search knowledge" }),
      "sqlite",
    );
    expect(screen.getByText("use sqlite cache")).toBeInTheDocument();
    expect(screen.queryByText("run preflight")).toBeNull();
  });

  it("a search inside a deep-linked project offers 'Clear search', not 'Clear project filter'", async () => {
    // The project HAS a lesson, but a search hides it — so the search is the
    // real cause, and "Clear project filter" would not fix it.
    lessonListSpy.mockResolvedValue([
      { ...memory, project_path: "/proj/app", content: "real lesson" },
    ]);
    listDecisionsSpy.mockResolvedValue([]);
    listEvidenceSpy.mockResolvedValue([]);
    const user = userEvent.setup();
    render(<KnowView initialProjectFilter="/proj/app" onReview={vi.fn()} />);
    await screen.findByText("real lesson");

    await user.type(
      screen.getByRole("textbox", { name: "Search knowledge" }),
      "zzznomatch",
    );
    expect(screen.getByRole("button", { name: "Clear search" })).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Clear project filter" }),
    ).toBeNull();
  });

  it("clearing the deep-link carrier resets the project filter (no stale hidden filter)", async () => {
    lessonListSpy.mockResolvedValue([
      { ...memory, id: "a", project_path: "/proj/a", content: "alpha lesson" },
      { ...memory, id: "b", project_path: "/proj/b", content: "beta lesson" },
    ]);
    listDecisionsSpy.mockResolvedValue([]);
    listEvidenceSpy.mockResolvedValue([]);
    const { rerender } = render(
      <KnowView initialProjectFilter="/proj/a" onReview={vi.fn()} />,
    );
    // Filtered to /proj/a — beta is hidden.
    expect(await screen.findByText("alpha lesson")).toBeInTheDocument();
    expect(screen.queryByText("beta lesson")).toBeNull();

    // Carrier cleared (a plain Know-tab click) → filter resets, beta returns.
    rerender(<KnowView initialProjectFilter={null} onReview={vi.fn()} />);
    expect(await screen.findByText("beta lesson")).toBeInTheDocument();
    expect(screen.getByText("alpha lesson")).toBeInTheDocument();
  });
});
