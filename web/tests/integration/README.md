# Integration tests

Postgres-backed tests for code paths that can't be exercised by the
unit suite (anything touching the DB or composing schema-aware
behavior end-to-end).

## How they run

Each test file calls `requireHarness()` from `./db.ts`. If
`TEST_DATABASE_URL` is unset in the environment, the test prints a
`SKIP` line and `process.exit(0)` — so a contributor without a test
database can still run `pnpm test:integration` without failures.

CI sets `TEST_DATABASE_URL` to a Neon preview-branch URL so the
tests actually exercise the DB. Skipping silently in CI would defeat
the purpose; CI should fail the build if the SKIP path fires (TODO:
add a `--require-db` flag to the harness).

## Setup

1. Create a fresh Postgres for testing — a Neon preview branch is
   the easiest. Local Postgres works too (`docker run postgres` or
   Postgres.app).

2. Apply migrations:

   ```bash
   TEST_DATABASE_URL=<your-test-db-url> \
     pnpm drizzle-kit push --config=drizzle.config.ts
   ```

   (Or temporarily set `DATABASE_URL=$TEST_DATABASE_URL` and run the
   normal push; drizzle-kit reads `DATABASE_URL`.)

3. Run the integration tests:

   ```bash
   TEST_DATABASE_URL=<your-test-db-url> pnpm test:integration
   ```

The harness's `resetTables()` truncates all moderation-relevant
tables (notifications, moderation_log, policy_decisions, flags,
comments, submission_tags, submissions, users) between each test
file's setup, but **keeps** the policy-moderator system user from
migration 0018. Tests that need that row call
`ensurePolicyModeratorUser()` defensively.

## What's covered today

| Test file | Verifies |
|---|---|
| `createSubmission-moderation.test.ts` | The pass / reject / synthetic-error / synthetic-capped branches of createSubmission, including the policy_decisions, moderation_log, and notifications side-effects |
| `createComment-moderation.test.ts` | The pass / non-illegal reject (optimistic publish) / illegal hard-block / synthetic-error (fail-open + retro enqueue) branches of createComment, including ban-candidate flag insertion (target_type='user'), notification appeal_url=null on illegal blocks, and retro_queue entry inserted on fail-open |

Add additional integration files for:

- `runCommentConfirmation.test.ts` — pass-2 retract path
- `drainRetroQueue.test.ts` — cron drain semantics, including the
  branch where 'disabled' / 'exempt' / 'capped' synthetic verdicts
  on the retry mark the queue entry 'done' rather than cycling
- `submitAppeal.test.ts` — race + dedup against the
  partial unique index from migration 0019

## Test-injection seam

The moderator's `moderate()` function reads a module-level override
when `process.env.NODE_ENV !== "production"`. Tests use:

```ts
import { __setTestVerdictOverride } from "@/lib/moderation";
__setTestVerdictOverride(myFakeVerdict);
try {
  await createSubmission(...);
} finally {
  __setTestVerdictOverride(null);
}
```

This avoids hitting OpenAI from tests while still exercising the
caller's verdict-handling logic. Setter throws if called in
production as a sanity net.

## CI integration (TODO)

`.github/workflows/ci-web.yml` currently only runs `pnpm test`. To
gate integration tests, add a job:

```yaml
- name: Integration tests
  env:
    TEST_DATABASE_URL: ${{ secrets.NEON_TEST_BRANCH_URL }}
  run: |
    pnpm drizzle-kit push --config=drizzle.config.ts
    pnpm test:integration
```

The Neon test branch URL needs creating once and storing as a
GitHub secret. Branch reset between runs is each test's
responsibility (`resetTables()`), not the URL provisioner.
