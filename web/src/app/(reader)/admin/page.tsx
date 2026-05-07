import Link from "next/link";

import { staffGate } from "@/lib/staff-gate";
import { getCurrentUser } from "@/lib/auth-shim";
import { auth } from "@/lib/auth";
import { HeaderChips, type InboxFilter } from "@/components/admin/HeaderChips";
import { NoticeStrip } from "@/components/admin/NoticeStrip";
import { InboxStream } from "@/components/admin/InboxStream";
import { InboxKeyboard } from "@/components/admin/InboxKeyboard";

const VALID_FILTERS: ReadonlySet<InboxFilter> = new Set([
  "all",
  "submission",
  "flag",
  "appeal",
]);

/**
 * /admin — Today inbox.
 *
 * Single-pane triage feed of pending submissions, open community
 * flags, and open appeals on AI rejects. Pending tag-vocabulary
 * proposals surface as a notice strip above the stream so they
 * don't fragment the time-ordered content feed.
 *
 * The legacy /admin/queue page is gone; redirect any old links to
 * /admin (handled by `redirects()` in next.config.mjs would also
 * work, but we just inlined the queue's logic here).
 */
export default async function AdminToday({
  searchParams,
}: {
  searchParams: Promise<{ as?: string; kind?: string }>;
}) {
  const sp = await searchParams;
  const gate = await staffGate(sp);
  if (gate) return gate;

  const asSuffix = sp.as ? `?as=${sp.as}` : "";
  const filterRaw = sp.kind;
  const filter: InboxFilter =
    filterRaw && VALID_FILTERS.has(filterRaw as InboxFilter)
      ? (filterRaw as InboxFilter)
      : "all";

  // Greeting line — pulls from the real session if present, else
  // the dev shim, else a generic fallback. Keeps the page useful
  // when the operator has typed `/admin` directly without
  // signing in via the browser flow.
  const session = await auth();
  const devUser = getCurrentUser(sp);
  const username = session?.user?.username ?? devUser?.username ?? null;

  return (
    <section className="proto-inbox">
      <header className="proto-inbox-header">
        <h1 className="proto-inbox-title">Today</h1>
        {username ? (
          <p className="proto-inbox-greeting">@{username}</p>
        ) : null}
      </header>

      <HeaderChips asSuffix={asSuffix} activeFilter={filter} />

      <NoticeStrip asSuffix={asSuffix} />

      <InboxStream asSuffix={asSuffix} filter={filter} />

      <footer className="proto-inbox-footer">
        <span className="proto-inbox-footer-label">Console →</span>
        <Link href={`/admin/console${asSuffix}`}>index</Link>
        <Link href={`/admin/console/vocabulary${asSuffix}`}>vocabulary</Link>
        <Link href={`/admin/console/policy${asSuffix}`}>policy</Link>
        <Link href={`/admin/console/users${asSuffix}`}>users</Link>
        <span className="proto-inbox-footer-sep">·</span>
        <Link href="/admin/log">Public log</Link>
        <span className="proto-inbox-footer-sep">·</span>
        <span className="proto-inbox-footer-hint">
          Press <kbd>?</kbd> for shortcuts
        </span>
      </footer>

      <InboxKeyboard />
    </section>
  );
}
