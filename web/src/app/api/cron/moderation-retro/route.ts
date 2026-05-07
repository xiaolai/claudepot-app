import { NextResponse } from "next/server";
import { withErrorHandling } from "@/lib/api/response";

import { drainRetroQueue } from "@/lib/moderation";

/**
 * Drain the retroactive moderation queue.
 *
 * Comments that fail-open on a model error are queued by
 * createComment for re-evaluation. This cron picks up to 25
 * 'pending' rows under SELECT … FOR UPDATE SKIP LOCKED, re-runs
 * moderate() against each, and (on reject) retracts the comment
 * via the same path the pass-2 confirmation uses.
 *
 * Idempotent across concurrent invocations; safe to run more often
 * than the schedule. Auth: same Bearer-token / CRON_SECRET shape
 * as the other cron routes.
 *
 * Schedule: every 5 minutes (vercel.json) so a fail-open comment
 * doesn't sit unmoderated for long. The cap on attempts (3) means
 * persistent model outages don't accumulate work indefinitely —
 * an entry transitions to 'failed' and stops being picked up.
 */
export const GET = withErrorHandling(async (req: Request) => {
  const expected = process.env.CRON_SECRET;
  const isProd = process.env.NODE_ENV === "production";
  if (isProd) {
    if (!expected) {
      return NextResponse.json(
        { error: "CRON_SECRET not configured" },
        { status: 500 },
      );
    }
    if (req.headers.get("authorization") !== `Bearer ${expected}`) {
      return NextResponse.json({ error: "unauthorized" }, { status: 401 });
    }
  } else if (expected) {
    if (req.headers.get("authorization") !== `Bearer ${expected}`) {
      return NextResponse.json({ error: "unauthorized" }, { status: 401 });
    }
  }

  const result = await drainRetroQueue();
  return NextResponse.json(result);
});
