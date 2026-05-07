import { NextResponse } from "next/server";

import { getSubmissionsByHot } from "@/db/queries";
import { escapeXml as escape } from "@/lib/escape-xml";

const SITE_URL = process.env.NEXT_PUBLIC_SITE_URL ?? "https://claudepot.com";

export async function GET() {
  const items = await getSubmissionsByHot(null, 30);
  const updated = items[0]?.submitted_at ?? new Date().toISOString();

  const xml = `<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>ClauDepot · hot</title>
  <link href="${SITE_URL}/" />
  <link rel="self" href="${SITE_URL}/api/rss" />
  <updated>${updated}</updated>
  <id>${SITE_URL}/</id>
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
