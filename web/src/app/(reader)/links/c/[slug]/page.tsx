import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";

import { loadCategoryPage } from "@/lib/links/queries";
import { LinkEntry } from "../../_components/LinkEntry";

export const revalidate = 3600;

type Props = { params: Promise<{ slug: string }> };

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const { slug } = await params;
  const data = await loadCategoryPage(slug);
  if (!data) return { title: "Not found" };
  return {
    title: `${data.category.name} · Links`,
    description: data.category.description ?? undefined,
  };
}

export default async function CategoryPage({ params }: Props) {
  const { slug } = await params;
  const data = await loadCategoryPage(slug);
  if (!data) notFound();

  const { category, parent, children, links } = data;

  return (
    <div className="cat-page">
      <nav className="cat-page-crumbs" aria-label="Breadcrumb">
        <Link href="/links">Links</Link>
        {parent ? (
          <>
            {" / "}
            <Link href={`/links/c/${parent.slug}`}>{parent.name}</Link>
          </>
        ) : null}
        {" / "}
        <span>{category.name}</span>
      </nav>

      <h1>{category.name}</h1>
      {category.description ? (
        <p className="cat-page-desc">{category.description}</p>
      ) : null}

      {children.length > 0 ? (
        <p className="cat-page-desc">
          Subcategories:{" "}
          {children.map((c, i) => (
            <span key={c.id}>
              {i > 0 ? " · " : null}
              <Link href={`/links/c/${c.slug}`}>{c.name}</Link>
            </span>
          ))}
        </p>
      ) : null}

      <ul className="link-list">
        {links.map((l) => (
          <LinkEntry key={l.id} link={l} />
        ))}
      </ul>
    </div>
  );
}
