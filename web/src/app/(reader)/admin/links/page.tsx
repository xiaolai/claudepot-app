import type { Metadata } from "next";
import Link from "next/link";

import "@/styles/links.css";
import { staffGate } from "@/lib/staff-gate";
import { loadPendingSuggestions } from "@/lib/links/queries";
import {
  approveLinkAction,
  rejectLinkAction,
} from "@/lib/actions/links";

export const metadata: Metadata = {
  title: "Links queue · Admin",
};

// Don't cache — staff need fresh state every refresh.
export const dynamic = "force-dynamic";

type Props = {
  searchParams: Promise<{ as?: string }>;
};

export default async function LinksQueuePage({ searchParams }: Props) {
  const sp = await searchParams;
  const gate = await staffGate(sp);
  if (gate) return gate;

  const pending = await loadPendingSuggestions();

  return (
    <section className="proto-inbox">
      <header className="proto-inbox-header">
        <h1 className="proto-inbox-title">Links queue</h1>
        <p className="proto-inbox-greeting">
          {pending.length} pending suggestion{pending.length === 1 ? "" : "s"}
        </p>
      </header>

      {pending.length === 0 ? (
        <p className="proto-empty proto-empty-spaced">
          The queue is empty. Reader suggestions will land here for review.
        </p>
      ) : (
        <ul className="links-queue">
          {pending.map(({ link, suggester }) => (
            <li key={link.id} className="links-queue-row">
              <header className="links-queue-meta">
                <a
                  href={link.url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="links-queue-name"
                >
                  {link.name}
                </a>
                <span className="links-queue-cat">
                  {link.primaryCategorySlug}
                </span>
                <span className="links-queue-time">
                  {new Date(link.createdAt).toLocaleString(undefined, {
                    month: "short",
                    day: "numeric",
                    hour: "2-digit",
                    minute: "2-digit",
                  })}
                </span>
                {suggester ? (
                  <span className="links-queue-by">
                    by{" "}
                    {suggester.username ? (
                      <Link href={`/u/${suggester.username}`}>
                        @{suggester.username}
                      </Link>
                    ) : (
                      (suggester.name ?? "unknown")
                    )}
                  </span>
                ) : null}
              </header>

              <div className="links-queue-url">{link.url}</div>
              {link.description ? (
                <p className="links-queue-desc">{link.description}</p>
              ) : null}

              <div className="links-queue-actions">
                <form action={approveLinkAction}>
                  <input type="hidden" name="id" value={link.id} />
                  <button type="submit" className="links-queue-approve">
                    Approve
                  </button>
                </form>
                <form action={rejectLinkAction}>
                  <input type="hidden" name="id" value={link.id} />
                  <button type="submit" className="links-queue-reject">
                    Reject
                  </button>
                </form>
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
