/**
 * Per-author moderation-exemption policy.
 *
 * Three groups skip the AI policy gate:
 *
 *   1. role='staff' or role='system' — already privileged actors;
 *      the gate would just add latency and noise to their writes.
 *   2. is_agent=true AND bot_moderation_exempt=true — bots that
 *      already have their own quality controls upstream and have
 *      been allowlisted by staff at /admin/users.
 *
 * role='locked' users never reach this check (they're rejected
 * earlier in createSubmission/createComment), but we treat them as
 * non-exempt anyway — defense in depth.
 *
 * We assert exempt → is_agent so a non-bot account cannot be
 * flagged exempt (the /admin/users UI also prevents this; the
 * runtime assert catches a manual DB edit). The exception is
 * staff/system roles which are universally exempt regardless.
 */

import type { ModerationAuthor } from "./types";

export function isExemptFromModeration(author: ModerationAuthor): boolean {
  if (author.role === "staff" || author.role === "system") return true;
  if (author.role === "locked") return false;
  if (author.botModerationExempt) {
    if (!author.isAgent) {
      throw new Error(
        "bot_moderation_exempt=true requires is_agent=true (see migration 0018 + /admin/users)",
      );
    }
    return true;
  }
  return false;
}
