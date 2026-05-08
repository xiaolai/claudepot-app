import Link from "next/link";

import type { Link as LinkRow, LinkCategory } from "@/db/schema/links";
import { LinkEntry } from "./LinkEntry";

type Props = {
  category: LinkCategory & { children: LinkCategory[] };
  links: Map<string, LinkRow[]>;
};

export function CategoryBlock({ category, links }: Props) {
  const childLinks = category.children.flatMap(
    (c) => links.get(c.slug) ?? [],
  );
  const directLinks = links.get(category.slug) ?? [];
  const total = childLinks.length + directLinks.length;

  if (total === 0) return null;

  return (
    <section
      id={category.slug}
      className="cat-block"
      data-category={category.slug}
    >
      <header className="cat-header">
        <h2>
          {category.name}
          <span className="cat-count">{total}</span>
        </h2>
        <Link href={`/links/c/${category.slug}`}>show all →</Link>
      </header>

      {category.children.length > 0 ? (
        category.children.map((child) => {
          const items = links.get(child.slug) ?? [];
          if (items.length === 0) return null;
          return (
            <div className="sub-block" key={child.id}>
              <h3>
                <Link href={`/links/c/${child.slug}`}>{child.name}</Link>
              </h3>
              <ul className="link-list">
                {items.map((l) => (
                  <LinkEntry key={l.id} link={l} />
                ))}
              </ul>
            </div>
          );
        })
      ) : (
        <ul className="link-list">
          {directLinks.map((l) => (
            <LinkEntry key={l.id} link={l} />
          ))}
        </ul>
      )}
    </section>
  );
}
