import type { ReactNode } from "react";
import { SubmissionRow } from "@/components/prototype/SubmissionRow";
import {
  AccountSidebar,
  type AccountSidebarPage,
} from "@/components/prototype/AccountSidebar";
import { auth } from "@/lib/auth";
import type { Submission } from "@/lib/prototype-fixtures";
import { getCurrentUser } from "@/lib/auth-shim";

/** Shared shell for the per-user feed pages in the personal hub
 *  (`/saved`, `/upvoted`, `/pending`). Resolves the viewer via real
 *  auth, falling back to the dev `?as=` shim. The shim is null in
 *  prod by contract — see `getCurrentUser` in prototype-fixtures.ts.
 *  Renders either a sign-in prompt (narrow layout) or the
 *  sidebar-flanked feed of submissions returned by the loader. */
export async function PersonalHubFeed({
  searchParams,
  current,
  title,
  dek,
  signedOutDek,
  emptyText,
  loader,
  rowMarkers,
}: {
  searchParams: { as?: string };
  current: AccountSidebarPage;
  title: string;
  dek: ReactNode;
  signedOutDek: ReactNode;
  emptyText: ReactNode;
  loader: (username: string) => Promise<Submission[]>;
  rowMarkers?: { initialSaved?: boolean; initialVote?: "up" | "down" | null };
}) {
  const session = await auth();
  const devUser = getCurrentUser(searchParams);
  const username = session?.user?.username ?? devUser?.username ?? null;

  if (!username) {
    return (
      <div className="proto-page-narrow">
        <h1>{title}</h1>
        <p className="proto-dek">{signedOutDek}</p>
        <p className="proto-empty proto-empty-spaced">
          Tip: append <code>?as=ada</code> to URLs to simulate a signed-in
          session.
        </p>
      </div>
    );
  }

  const items = await loader(username);

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
      </div>
    </div>
  );
}
