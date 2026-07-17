// Cross-section deep-link handoff for the Keys filter.
//
// AccountsSection's "N tokens" chip navigates to the (lazy-loaded)
// KeysSection pre-filtered to an account email. A CustomEvent alone
// drops on first navigation — the section's chunk hasn't mounted its
// listener yet when the event fires (audit 2026-07 F4). This
// module-level pending slot survives until KeysSection mounts and
// consumes it; the CustomEvent path still covers an already-mounted
// KeysSection, whose handler clears the slot so it can't go stale.

let pending: string | null = null;

/** Stage a filter query for the next KeysSection mount. */
export function setPendingKeysFilter(query: string): void {
  pending = query;
}

/** Read-and-clear the staged query. Returns null when none staged. */
export function consumePendingKeysFilter(): string | null {
  const p = pending;
  pending = null;
  return p;
}
