import type { Link } from "@/db/schema/links";

export function LinkEntry({ link }: { link: Link }) {
  return (
    <li className="link-entry" data-region={link.region ?? undefined}>
      <a href={link.url} target="_blank" rel="noopener noreferrer" title={link.url}>
        <span className="link-name">{link.name}</span>
        {link.description ? (
          <span className="link-desc">— {link.description}</span>
        ) : null}
        {link.region === "cn" ? <span className="link-region-tag">CN</span> : null}
      </a>
    </li>
  );
}
