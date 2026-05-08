/**
 * Read-only post-apply check for migration 0036.
 * Run via: pnpm exec tsx --env-file=.env.local scripts/verify-0036.ts
 *
 * Confirms each load-bearing artifact landed:
 *   - 'draft' is in the content_state enum
 *   - idx_decision_records_idempotency exists and is UNIQUE
 *   - comments.is_meta exists with NOT NULL DEFAULT false
 *   - override_records.reviewer_kind exists with the new enum
 *   - engagement_records table + 2 indexes exist
 */

import { neon } from "@neondatabase/serverless";

const url = process.env.DATABASE_URL ?? process.env.NEON_DATABASE_URL;
if (!url) {
  console.error("missing DATABASE_URL");
  process.exit(1);
}
const sql = neon(url);

let pass = 0;
let fail = 0;
function check(name: string, cond: boolean, detail = "") {
  if (cond) {
    console.log(`PASS  ${name}${detail ? " — " + detail : ""}`);
    pass += 1;
  } else {
    console.error(`FAIL  ${name}${detail ? " — " + detail : ""}`);
    fail += 1;
  }
}

const enumValues = (await sql.query(
  "SELECT unnest(enum_range(NULL::content_state))::text AS v",
)) as Array<{ v: string }>;
const states = enumValues.map((r) => r.v);
check("content_state has 'draft'", states.includes("draft"), states.join(","));

const idem = (await sql.query(
  "SELECT indexdef FROM pg_indexes WHERE schemaname='public' AND indexname='idx_decision_records_idempotency'",
)) as Array<{ indexdef: string }>;
check("idx_decision_records_idempotency exists", idem.length === 1);
check(
  "idx_decision_records_idempotency is UNIQUE",
  idem[0]?.indexdef.toUpperCase().includes("UNIQUE INDEX") ?? false,
);

const isMeta = (await sql.query(
  "SELECT column_name, is_nullable, column_default FROM information_schema.columns WHERE table_name='comments' AND column_name='is_meta'",
)) as Array<{
  column_name: string;
  is_nullable: string;
  column_default: string | null;
}>;
check("comments.is_meta exists", isMeta.length === 1);
check(
  "comments.is_meta is NOT NULL",
  isMeta[0]?.is_nullable === "NO",
  isMeta[0]?.is_nullable,
);
check(
  "comments.is_meta default is false",
  (isMeta[0]?.column_default ?? "").includes("false"),
  isMeta[0]?.column_default ?? "<null>",
);

const partial = (await sql.query(
  "SELECT indexdef FROM pg_indexes WHERE schemaname='public' AND indexname='idx_comments_submission_visible_nonmeta'",
)) as Array<{ indexdef: string }>;
check("idx_comments_submission_visible_nonmeta exists", partial.length === 1);

const reviewerKind = (await sql.query(
  "SELECT column_name, data_type, udt_name, is_nullable FROM information_schema.columns WHERE table_name='override_records' AND column_name='reviewer_kind'",
)) as Array<{
  column_name: string;
  udt_name: string;
  is_nullable: string;
}>;
check("override_records.reviewer_kind exists", reviewerKind.length === 1);
check(
  "override_records.reviewer_kind uses reviewer_kind enum",
  reviewerKind[0]?.udt_name === "reviewer_kind",
  reviewerKind[0]?.udt_name,
);

const engagementTable = (await sql.query(
  "SELECT table_name FROM information_schema.tables WHERE table_schema='public' AND table_name='engagement_records'",
)) as Array<{ table_name: string }>;
check("engagement_records table exists", engagementTable.length === 1);

const engagementCols = (await sql.query(
  "SELECT column_name FROM information_schema.columns WHERE table_name='engagement_records' ORDER BY ordinal_position",
)) as Array<{ column_name: string }>;
const expected = [
  "id",
  "submission_id",
  "kind",
  "actor_id",
  "occurred_at",
  "metadata",
];
const cols = engagementCols.map((r) => r.column_name);
check(
  "engagement_records columns match expected set",
  expected.every((c) => cols.includes(c)),
  cols.join(","),
);

const engagementIdx = (await sql.query(
  "SELECT indexname FROM pg_indexes WHERE schemaname='public' AND tablename='engagement_records' ORDER BY indexname",
)) as Array<{ indexname: string }>;
const idxNames = engagementIdx.map((r) => r.indexname);
check(
  "engagement_records (submission, occurred_at) index exists",
  idxNames.includes("idx_engagement_records_submission_occurred"),
);
check(
  "engagement_records (actor, occurred_at) index exists",
  idxNames.includes("idx_engagement_records_actor_occurred"),
);

console.log(`\n${pass} passed, ${fail} failed`);
process.exit(fail > 0 ? 1 : 0);
