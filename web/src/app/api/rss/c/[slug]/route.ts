import { NextResponse } from "next/server";

import { getSubmissionsByTag, getTagBySlug } from "@/db/queries";
import { escapeXml as escape } from "@/lib/escape-xml";

const SITE_URL = process.env.NEXT_PUBLIC_SITE_URL ?? "https://claudepot.com";

export async function GET(
  _req: Request,
  { params }: { params: Promise<{ slug: string }> },
) {
  const { slug } = await params;
  const tag = await getTagBySlug(slug);
  if (!tag) return new NextResponse("Not found", { status: 404 });

  const items = (await getSubmissionsByTag(slug)).slice(0, 30);
  const updated = items[0]?.submitted_at ?? new Date().toISOString();

  const xml = `<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>ClauDepot · #${escape(tag.name)}</title>
  <link href="${SITE_URL}/c/${slug}" />
  <link rel="self" href="${SITE_URL}/api/rss/c/${slug}" />
  <updated>${updated}</updated>
  <id>${SITE_URL}/c/${slug}</id>
${items
  .map(
    (s) => `  <entry>
    <title>${escape(s.title)}</title>
    <link href="${SITE_URL}/post/${s.id}" />
    <id>${SITE_URL}/post/${s.id}</id>
    <updated>${s.submitted_at}</updated>
    <author><name>${escape(s.user)}</name></author>
    <summary>${escape(s.text ?? s.url ?? "")}</summary>
  </entry>`,
  )
  .join("\n")}
</feed>`;

  return new NextResponse(xml, {
    headers: { "content-type": "application/atom+xml; charset=utf-8" },
  });
}
