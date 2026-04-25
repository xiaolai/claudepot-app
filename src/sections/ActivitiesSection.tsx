import { SessionsSection, type SessionsSectionProps } from "./SessionsSection";

/**
 * Activities section — cross-project session firehose.
 *
 * The dashboard strip that previously sat above the session list has
 * moved to the (renamed) Activity surface (`Events` tab → relabeled
 * "Activity"), where it pairs with the activity-cards stream as a
 * unified "what's happening / has happened" view. This section now
 * exists only to host SessionsSection until per-project session
 * browsing inside `Projects → ProjectDetail` is verified competent
 * for power-user flows (cross-project search, repo grouping, the
 * Cleanup sub-tab); at that point the section can be removed and
 * its sub-tabs relocated.
 *
 * Kept as a thin wrapper — SessionsSection has substantial internal
 * state (filter store, live strip, selection) that we don't want to
 * mount at multiple call sites concurrently.
 */
export function ActivitiesSection(props: SessionsSectionProps = {}) {
  return <SessionsSection {...props} />;
}
