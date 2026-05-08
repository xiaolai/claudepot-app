import type { Link } from "@/db/schema/links";

export function FeaturedStrip({ links }: { links: Link[] }) {
  if (links.length === 0) return null;
  return (
    <section className="featured-strip" aria-label="Editor's picks">
      {links.map((l) => (
        <a
          key={l.id}
          className="featured-card"
          href={l.url}
          target="_blank"
          rel="noopener noreferrer"
        >
          <h3>{l.name}</h3>
          <p>{l.featuredBlurb ?? l.description}</p>
        </a>
      ))}
    </section>
  );
}
