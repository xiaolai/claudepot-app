// Phase 5: bridge `useStatusIssues` (which derives a banner list
// from app state on every render) into the notification log via
// emit(). Emits a P0 category entry the first time each issue id
// appears; emits a P2 `bannerResolved` entry when the id leaves.
//
// Memory-only state: the ref resets on renderer reload. Trade-off:
// a reload during an unresolved banner re-emits the "first show"
// event once. The category-keyed dedupe inside the OS dispatcher's
// token bucket caps the noise; one extra log entry per reload is
// acceptable (and the plan doc walked through this exact decision).

import { useEffect, useRef } from "react";
import { useEmit } from "../providers/AppStateProvider";
import type { Category } from "../lib/notifications/types";
import type { StatusIssue } from "./useStatusIssues";

/** Map `useStatusIssues`'s string ids to a routing Category. The
 *  ids are stable strings the hook uses internally; the mapping
 *  here is the single source-of-truth for "which banner category
 *  is this banner?" Update this table when adding a new banner. */
function categoryForIssueId(id: string): Category | null {
  // `useStatusIssues` builds ids like `account:<email>:auth-rejected`,
  // `keychain-locked`, `cc-slot-drift`, `desktop-drift`, `repair-conflict`.
  // We match by prefix so per-account banners (auth-rejected on
  // each email) all map to the same routing category.
  if (id.startsWith("account:") && id.endsWith(":auth-rejected")) {
    return "accountAuthRejected";
  }
  if (id === "keychain-locked" || id.startsWith("keychain")) {
    return "keychainLocked";
  }
  if (id === "cc-slot-drift" || id.startsWith("cc-slot")) {
    return "ccSlotDrift";
  }
  if (id === "desktop-drift" || id.startsWith("desktop-drift")) {
    return "desktopDrift";
  }
  if (id === "repair-conflict" || id.startsWith("repair")) {
    return "repairConflict";
  }
  // Sync warnings, info banners, etc. don't have a P0 category — they
  // already surface via the banner itself; no log entry needed.
  return null;
}

/**
 * Watch the live `issues` list. On every state-transition emit a
 * routing event:
 *   - First time an id appears → P0 event (one per category)
 *   - Id leaves while previously present → P2 `bannerResolved`
 *
 * The hook is render-driven: `issues` comes from `useStatusIssues`
 * which recomputes on every relevant state change. Diffing against
 * the previous render's id set is enough — no extra subscribers.
 */
export function useStatusBannerEmits(issues: StatusIssue[]): void {
  const emit = useEmit();
  const previousIdsRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    const current = new Set(issues.map((i) => i.id));
    const previous = previousIdsRef.current;

    // First-show emits.
    for (const issue of issues) {
      if (previous.has(issue.id)) continue;
      const category = categoryForIssueId(issue.id);
      if (!category) continue;
      void emit({
        category,
        kind: issue.severity === "error" ? "error" : "notice",
        title: issue.label,
        body: issue.detail ?? "",
        dedupeKey: `banner:${issue.id}`,
      });
    }

    // Resolved emits — ids present last render, gone this render.
    for (const id of previous) {
      if (current.has(id)) continue;
      const category = categoryForIssueId(id);
      if (!category) continue;
      void emit({
        category: "bannerResolved",
        title: `Resolved: ${humanizeCategory(category)}`,
        body: "",
        dedupeKey: `banner-resolved:${id}`,
      });
    }

    previousIdsRef.current = current;
  }, [issues, emit]);
}

function humanizeCategory(category: Category): string {
  switch (category) {
    case "accountAuthRejected":
      return "account auth";
    case "keychainLocked":
      return "keychain access";
    case "ccSlotDrift":
      return "CC slot drift";
    case "desktopDrift":
      return "Desktop drift";
    case "repairConflict":
      return "repair conflict";
    default:
      return String(category);
  }
}
