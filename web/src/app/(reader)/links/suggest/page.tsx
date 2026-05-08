import type { Metadata } from "next";
import Link from "next/link";
import { asc, isNull } from "drizzle-orm";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { linkCategories } from "@/db/schema/links";
import { suggestLinkAction } from "@/lib/actions/links";

export const metadata: Metadata = {
  title: "Suggest a link",
  description:
    "Suggest a Claude/AI link for the curated directory at claudepot.com/links/.",
};

type Props = {
  searchParams: Promise<{
    status?: string;
    error?: string;
    url?: string;
    name?: string;
    description?: string;
  }>;
};

export default async function SuggestLinkPage({ searchParams }: Props) {
  const sp = await searchParams;
  const session = await auth();

  if (!session?.user) {
    return (
      <div className="cat-page">
        <nav className="cat-page-crumbs" aria-label="Breadcrumb">
          <Link href="/links">Links</Link> / <span>Suggest</span>
        </nav>
        <h1>Suggest a link</h1>
        <p className="cat-page-desc">
          <Link href="/login?callbackUrl=/links/suggest">Sign in</Link> to
          suggest a link for the curated directory.
        </p>
      </div>
    );
  }

  if (sp.status === "submitted") {
    return (
      <div className="cat-page">
        <nav className="cat-page-crumbs" aria-label="Breadcrumb">
          <Link href="/links">Links</Link> / <span>Suggest</span>
        </nav>
        <h1>Thanks — your suggestion is in the queue.</h1>
        <p className="cat-page-desc">
          A curator will review it before it appears on the directory.
        </p>
        <p>
          <Link href="/links/suggest">Suggest another</Link> ·{" "}
          <Link href="/links">Back to /links</Link>
        </p>
      </div>
    );
  }

  const topLevel = await db
    .select({ slug: linkCategories.slug, name: linkCategories.name })
    .from(linkCategories)
    .where(isNull(linkCategories.parentId))
    .orderBy(asc(linkCategories.displayOrder));

  return (
    <div className="cat-page">
      <nav className="cat-page-crumbs" aria-label="Breadcrumb">
        <Link href="/links">Links</Link> / <span>Suggest</span>
      </nav>
      <h1>Suggest a link</h1>
      <p className="cat-page-desc">
        Submissions land in the curator queue. Expect a review pass before they
        appear on the directory.
      </p>

      {sp.error ? (
        <p className="suggest-error" role="alert">
          {sp.error}
        </p>
      ) : null}

      <form action={suggestLinkAction} className="suggest-form">
        <label>
          <span>URL</span>
          <input
            type="url"
            name="url"
            required
            maxLength={500}
            defaultValue={sp.url ?? ""}
            placeholder="https://example.com/whatever"
            autoFocus
          />
        </label>

        <label>
          <span>Name</span>
          <input
            type="text"
            name="name"
            required
            maxLength={80}
            defaultValue={sp.name ?? ""}
            placeholder="Human-readable label, ≤80 chars"
          />
        </label>

        <label>
          <span>Description</span>
          <input
            type="text"
            name="description"
            maxLength={200}
            defaultValue={sp.description ?? ""}
            placeholder="One-line blurb, ≤200 chars"
          />
        </label>

        <label>
          <span>Category</span>
          <select name="primaryCategorySlug" required defaultValue="">
            <option value="" disabled>
              Pick a top-level category…
            </option>
            {topLevel.map((c) => (
              <option key={c.slug} value={c.slug}>
                {c.name}
              </option>
            ))}
          </select>
        </label>

        <div className="suggest-form-actions">
          <button type="submit">Submit suggestion</button>
          <Link href="/links">Cancel</Link>
        </div>
      </form>
    </div>
  );
}
