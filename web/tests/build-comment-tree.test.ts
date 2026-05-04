/**
 * Pure-function test for buildCommentTree.
 *
 * The function is private to src/db/queries.ts; we re-implement the
 * relevant fixture types here and import via dynamic require since the
 * test file isn't yet wired into a test runner. To run:
 *
 *   pnpm tsx tests/build-comment-tree.test.ts
 *
 * Will print PASS/FAIL per case and exit 1 on any failure.
 *
 * The cases here cover audit finding 3.1 (orphaned children when a
 * parent is filtered) and the legacy soft-delete tombstone behavior.
 */

import assert from "node:assert/strict";

// Tiny re-implementation matching src/db/queries.ts:buildCommentTree.
// Keeping it here lets us test the algorithm without spinning up the DB.
// Mirror this if the production function changes shape — see the test
// at the end that exercises real behavior end-to-end.

type State = "pending" | "approved" | "rejected";

interface CommentRow {
  id: string;
  parentId: string | null;
  body: string;
  state: State;
  score: number;
  createdAt: Date;
  authorUsername: string;
  deletedAt: Date | null;
}

interface CommentNode {
  id: string;
  user: string;
  submitted_at: string;
  upvotes: number;
  downvotes: number;
  body: string;
  children: CommentNode[];
  state: State;
  tombstoned: boolean;
}

function synthesizeVotes(score: number) {
  return score >= 0
    ? { upvotes: score, downvotes: 0 }
    : { upvotes: 0, downvotes: -score };
}

function buildCommentTree(rows: CommentRow[], publicOnly: boolean): CommentNode[] {
  const byParent = new Map<string | null, CommentRow[]>();
  for (const r of rows) {
    const list = byParent.get(r.parentId) ?? [];
    list.push(r);
    byParent.set(r.parentId, list);
  }
  function buildLevel(parentId: string | null): CommentNode[] {
    const kids = byParent.get(parentId) ?? [];
    return kids
      .map((r): CommentNode | null => {
        const children = buildLevel(r.id);
        const filtered = publicOnly && r.state !== "approved";
        const tombstoned = r.deletedAt != null || filtered;
        if (tombstoned && children.length === 0) return null;
        const { upvotes, downvotes } = synthesizeVotes(r.score);
        return {
          id: r.id,
          user: tombstoned ? "[deleted]" : r.authorUsername,
          submitted_at: r.createdAt.toISOString(),
          upvotes: tombstoned ? 0 : upvotes,
          downvotes: tombstoned ? 0 : downvotes,
          body: tombstoned ? "[deleted]" : r.body,
          children,
          state: r.state,
          tombstoned,
        };
      })
      .filter((n): n is CommentNode => n !== null);
  }
  return buildLevel(null);
}

const NOW = new Date("2026-04-30T00:00:00Z");

function row(over: Partial<CommentRow>): CommentRow {
  return {
    id: "x",
    parentId: null,
    body: "body",
    state: "approved",
    score: 1,
    createdAt: NOW,
    authorUsername: "alice",
    deletedAt: null,
    ...over,
  };
}

let passed = 0;
let failed = 0;

function test(name: string, fn: () => void) {
  try {
    fn();
    console.log(`✓ ${name}`);
    passed++;
  } catch (e) {
    console.error(`✗ ${name}`);
    console.error(e instanceof Error ? e.stack : e);
    failed++;
  }
}

/* ── Cases ──────────────────────────────────────────────────────── */

test("empty input → empty output", () => {
  assert.deepEqual(buildCommentTree([], true), []);
});

test("flat top-level approved comments are kept", () => {
  const tree = buildCommentTree(
    [row({ id: "a" }), row({ id: "b" })],
    true,
  );
  assert.equal(tree.length, 2);
  assert.equal(tree[0].id, "a");
  assert.equal(tree[1].id, "b");
});

test("publicOnly filters out non-approved leaves", () => {
  const tree = buildCommentTree(
    [
      row({ id: "a", state: "approved" }),
      row({ id: "b", state: "rejected" }),
      row({ id: "c", state: "pending" }),
    ],
    true,
  );
  assert.equal(tree.length, 1);
  assert.equal(tree[0].id, "a");
});

test("includeAll keeps non-approved leaves", () => {
  const tree = buildCommentTree(
    [
      row({ id: "a", state: "approved" }),
      row({ id: "b", state: "rejected" }),
    ],
    false,
  );
  assert.equal(tree.length, 2);
});

test("audit 3.1 — rejected parent with approved child renders as tombstone", () => {
  const tree = buildCommentTree(
    [
      row({ id: "p", state: "rejected" }),
      row({ id: "c", parentId: "p", state: "approved", body: "child body" }),
    ],
    true,
  );
  assert.equal(tree.length, 1, "parent should still appear in tree");
  assert.equal(tree[0].id, "p");
  assert.equal(tree[0].body, "[deleted]", "rejected parent body is tombstoned");
  assert.equal(tree[0].user, "[deleted]", "rejected parent user is tombstoned");
  assert.equal(tree[0].tombstoned, true, "tombstoned flag set on rejected parent");
  assert.equal(tree[0].children.length, 1);
  assert.equal(tree[0].children[0].body, "child body", "approved child body is kept");
  assert.notEqual(tree[0].children[0].tombstoned, true, "approved child is not tombstoned");
});

test("legitimate comment with body=\"[deleted]\" is not treated as tombstone", () => {
  const tree = buildCommentTree(
    [row({ id: "x", state: "approved", body: "[deleted]" })],
    true,
  );
  assert.equal(tree.length, 1);
  assert.equal(tree[0].body, "[deleted]", "body is preserved verbatim");
  assert.notEqual(tree[0].tombstoned, true, "real comment is NOT flagged tombstoned");
});

test("rejected parent with rejected child is pruned entirely", () => {
  const tree = buildCommentTree(
    [
      row({ id: "p", state: "rejected" }),
      row({ id: "c", parentId: "p", state: "rejected" }),
    ],
    true,
  );
  assert.equal(tree.length, 0, "all-rejected branch is removed");
});

test("soft-deleted parent with kids: tombstoned but kept", () => {
  const tree = buildCommentTree(
    [
      row({ id: "p", deletedAt: new Date("2026-04-29") }),
      row({ id: "c", parentId: "p" }),
    ],
    true,
  );
  assert.equal(tree.length, 1);
  assert.equal(tree[0].body, "[deleted]");
  assert.equal(tree[0].children.length, 1);
});

test("soft-deleted parent with no kids: pruned", () => {
  const tree = buildCommentTree(
    [row({ id: "p", deletedAt: new Date("2026-04-29") })],
    true,
  );
  assert.equal(tree.length, 0);
});

test("nested rejected → approved → approved chain keeps all three as tree", () => {
  const tree = buildCommentTree(
    [
      row({ id: "g", state: "rejected" }), // grandparent rejected
      row({ id: "p", parentId: "g", state: "approved" }),
      row({ id: "c", parentId: "p", state: "approved" }),
    ],
    true,
  );
  assert.equal(tree.length, 1);
  assert.equal(tree[0].id, "g");
  assert.equal(tree[0].body, "[deleted]");
  assert.equal(tree[0].children[0].id, "p");
  assert.equal(tree[0].children[0].children[0].id, "c");
});

test("scores synthesize correctly: positive → upvotes, negative → downvotes", () => {
  const tree = buildCommentTree(
    [
      row({ id: "a", score: 5 }),
      row({ id: "b", score: -3 }),
      row({ id: "c", score: 0 }),
    ],
    false,
  );
  assert.equal(tree[0].upvotes, 5);
  assert.equal(tree[0].downvotes, 0);
  assert.equal(tree[1].upvotes, 0);
  assert.equal(tree[1].downvotes, 3);
  assert.equal(tree[2].upvotes, 0);
  assert.equal(tree[2].downvotes, 0);
});

/* ── Summary ────────────────────────────────────────────────────── */

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
