import type { ReactNode } from "react";
import Link from "next/link";
import { SubmissionRow } from "@/components/prototype/SubmissionRow";
import {
  AccountSidebar,
  type AccountSidebarPage,
} from "@/components/prototype/AccountSidebar";
import { auth } from "@/lib/auth";
import type { Submission } from "@/lib/prototype-fixtures";
import type { Page } from "@/db/queries";
import {
  decodeCursor,
  isCursorTime,
  type CursorTime,
} from "@/lib/api/cursor";
import { getCurrentUser } from "@/lib/auth-shim";

/** Shared shell for the per-user feed pages in the personal hub
 *  (`/saved`, `/upvoted`, `/pending`). Resolves the viewer via real
 *  auth, falling back to the dev `?as=` shim. The shim is null in
 *  prod by contract — see `getCurrentUser` in prototype-fixtures.ts.
 *  Renders either a sign-in prompt (narrow layout) or the
 *  sidebar-flanked, paginated feed returned by the loader. */
export async function PersonalHubFeed({
  searchParams,
  current,
  title,
  dek,
  signedOutDek,
  emptyText,
  loader,
  rowMarkers,
  basePath,
}: {
  searchParams: { as?: string; cursor?: string };
  current: AccountSidebarPage;
  title: string;
  dek: ReactNode;
  signedOutDek: ReactNode;
  emptyText: ReactNode;
  loader: (
    username: string,
    opts: { cursor?: CursorTime | null },
  ) => Promise<Page<Submission>>;
  rowMarkers?: { initialSaved?: boolean; initialVote?: "up" | "down" | null };
  basePath: string;
}) {
  const session = await auth();
  const devUser = getCurrentUser(searchParams);
  const username = session?.user?.username ?? devUser?.username ?? null;

  if (!username) {
    const isDev = process.env.NODE_ENV !== "production";
    return (
      <div className="proto-page-narrow">
        <h1>{title}</h1>
        <p className="proto-dek">{signedOutDek}</p>
        {isDev ? (
          <p className="proto-empty proto-empty-spaced">
            Tip: append <code>?as=ada</code> to URLs to simulate a signed-in
            session.
          </p>
        ) : null}
      </div>
    );
  }

  const decoded = decodeCursor(searchParams.cursor);
  const cursor = decoded && isCursorTime(decoded) ? decoded : null;
  const { items, nextCursor } = await loader(username, { cursor });

  const olderHref =
    nextCursor &&
    `${basePath}?cursor=${encodeURIComponent(nextCursor)}${searchParams.as ? `&as=${searchParams.as}` : ""}`;

  return (
    <div className="proto-page-aside">
      <AccountSidebar
        current={current}
        username={username}
        asParam={searchParams.as}
      />
      <div className="proto-page-aside-content">
        <h1>{title}</h1>
        <p className="proto-dek">{dek}</p>
        <ol className="proto-feed">
          {items.length === 0 ? (
            <li className="proto-empty">{emptyText}</li>
          ) : (
            items.map((s) => (
              <SubmissionRow key={s.id} submission={s} {...rowMarkers} />
            ))
          )}
        </ol>
        {olderHref ? (
          <p className="proto-pagination">
            <Link href={olderHref}>Older →</Link>
          </p>
        ) : null}
      </div>
    </div>
  );
}
