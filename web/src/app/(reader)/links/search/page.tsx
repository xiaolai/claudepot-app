import type { Metadata } from "next";
import Link from "next/link";

import { searchLinks } from "@/lib/links/queries";

export const metadata: Metadata = {
  title: "Search · Links",
};

type Props = { searchParams: Promise<{ q?: string }> };

export default async function LinkSearchPage({ searchParams }: Props) {
  const { q = "" } = await searchParams;
  const results = q ? await searchLinks(q, 100) : [];

  return (
    <div className="search-page">
      <nav className="cat-page-crumbs" aria-label="Breadcrumb">
        <Link href="/links">Links</Link>
        {" / "}
        <span>Search</span>
      </nav>

      <h1>Search</h1>

      <form action="/links/search" method="GET" className="links-search">
        <input
          type="search"
          name="q"
          defaultValue={q}
          placeholder="Search links…"
          aria-label="Search links"
        />
      </form>

      <p className="search-meta">
        {q
          ? `${results.length} result${results.length === 1 ? "" : "s"} for “${q}”`
          : "Type to search 1,000+ Claude/AI links."}
      </p>

      {results.map((r) => (
        <article key={r.id} className="search-result">
          <h3>
            <a href={r.url} target="_blank" rel="noopener noreferrer">
              {r.name}
            </a>
          </h3>
          <p>{r.description}</p>
          <p className="search-result-meta">
            {r.url} · {r.primaryCategorySlug}
            {r.region ? ` · ${r.region}` : null}
          </p>
        </article>
      ))}
    </div>
  );
}
