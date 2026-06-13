import { useEffect, useRef } from "react";
import type { StatusIssue } from "./useStatusIssues";

/**
 * Snooze auto-clear, extracted from AppShell: when an issue id is no
 * longer present in `rawIssues`, the underlying condition has
 * resolved. Drop its entry from the dismissed-issues store so a
 * re-occurrence later shows the banner immediately instead of being
 * silently re-snoozed against the stale 24 h timer.
 *
 * The first effect run reconciles localStorage's dismissed-store
 * against the live rawIssues — this catches stale entries left
 * over from a previous renderer lifetime (user dismissed issue X,
 * closed app, condition resolved while closed, app reopened
 * before the 24 h TTL would expire X). Subsequent runs diff
 * against a ref of the previous-tick id set so we only call
 * `clearDismissed` for ids that actually disappeared this tick.
 */
export function useSnoozeAutoClear(args: {
  rawIssues: StatusIssue[];
  clearDismissed: (id: string) => void;
  knownDismissedKeys: () => string[];
}): void {
  const { rawIssues, clearDismissed, knownDismissedKeys } = args;
  const seenIssueIdsRef = useRef<Set<string> | null>(null);

  useEffect(() => {
    const current = new Set(rawIssues.map((i) => i.id));
    const prev = seenIssueIdsRef.current;
    if (prev === null) {
      // First run — reconcile against persisted snooze entries from
      // a prior renderer lifetime, not just the in-memory ref.
      for (const id of knownDismissedKeys()) {
        if (!current.has(id)) clearDismissed(id);
      }
    } else {
      for (const id of prev) {
        if (!current.has(id)) clearDismissed(id);
      }
    }
    seenIssueIdsRef.current = current;
  }, [rawIssues, clearDismissed, knownDismissedKeys]);
}
