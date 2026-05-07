#!/usr/bin/env node
// One-shot cleanup: drop the original fictional submissions/comments, then
// recompute human karma from only the remaining (real) submissions.
//
// Drops:
//   - submissions with numeric id <= 40
//   - submissions with id starting with "p-" (the legacy moderation demos —
//     redundant now that the import generates 6 pending + 6 rejected)
//   - comment threads keyed by any of the above

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const ROOT = path.resolve(path.dirname(__filename), "../..");
const FIXTURES = path.join(ROOT, "design/fixtures");
const SUBMISSIONS_FILE = path.join(FIXTURES, "submissions.json");
const COMMENTS_FILE = path.join(FIXTURES, "comments.json");
const USERS_FILE = path.join(FIXTURES, "users.json");

const readJson = (f) => JSON.parse(fs.readFileSync(f, "utf8"));
const writeJson = (f, d) => fs.writeFileSync(f, JSON.stringify(d, null, 2) + "\n");

// Pre-import karma baselines (from the original users.json before this work).
// These represent hypothetical prior site activity — comments, votes, etc. —
// NOT the specific dropped submissions, so they stay.
const BASELINE_KARMA = {
  ada: 1284,
  kai: 873,
  miro: 612,
  lin: 459,
  sasha: 401,
  zed: 318,
  nova: 287,
  ren: 142,
  ish: 89,
  lixiaolai: 0,
  ClauDepot: 0,
};

const KARMA_PER_SUBMISSION = 12;

function isOriginal(id) {
  if (typeof id !== "string") return false;
  if (id.startsWith("p-")) return true;
  const n = Number(id);
  return Number.isFinite(n) && n <= 40;
}

function main() {
  const subs = readJson(SUBMISSIONS_FILE);
  const comments = readJson(COMMENTS_FILE);
  const users = readJson(USERS_FILE);

  const before = { subs: subs.length, comments: Object.keys(comments).length };

  // Drop originals
  const keptSubs = subs.filter((s) => !isOriginal(s.id));
  const keptComments = Object.fromEntries(
    Object.entries(comments).filter(([key]) => !isOriginal(key)),
  );

  // Recompute karma: baseline + 12 × current submission count
  const submissionCount = {};
  for (const s of keptSubs) {
    submissionCount[s.user] = (submissionCount[s.user] ?? 0) + 1;
  }
  for (const u of users) {
    if (u.is_system) continue;
    const baseline = BASELINE_KARMA[u.username] ?? 0;
    const bump = (submissionCount[u.username] ?? 0) * KARMA_PER_SUBMISSION;
    u.karma = baseline + bump;
  }

  writeJson(SUBMISSIONS_FILE, keptSubs);
  writeJson(COMMENTS_FILE, keptComments);
  writeJson(USERS_FILE, users);

  console.log("Submissions:", before.subs, "→", keptSubs.length, `(dropped ${before.subs - keptSubs.length})`);
  console.log("Comment threads:", before.comments, "→", Object.keys(keptComments).length, `(dropped ${before.comments - Object.keys(keptComments).length})`);
  console.log("\nRecomputed karma:");
  for (const u of users) {
    if (u.is_system) continue;
    console.log(`  ${u.username.padEnd(12)} ${u.karma}  (${submissionCount[u.username] ?? 0} submissions × ${KARMA_PER_SUBMISSION} + ${BASELINE_KARMA[u.username] ?? 0} baseline)`);
  }
}

main();
