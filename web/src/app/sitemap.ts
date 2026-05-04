import type { MetadataRoute } from "next";

import { getAllSubmissions, getAllTags } from "@/db/queries";

const SITE_URL = process.env.NEXT_PUBLIC_SITE_URL ?? "https://claudepot.com";

export default async function sitemap(): Promise<MetadataRoute.Sitemap> {
  const [submissions, tags] = await Promise.all([
    getAllSubmissions(),
    getAllTags(),
  ]);
  const now = new Date();
  return [
    { url: `${SITE_URL}/`, lastModified: now, changeFrequency: "hourly", priority: 1.0 },
    { url: `${SITE_URL}/new`, lastModified: now, changeFrequency: "hourly", priority: 0.9 },
    { url: `${SITE_URL}/top`, lastModified: now, changeFrequency: "daily", priority: 0.8 },
    { url: `${SITE_URL}/c`, lastModified: now, changeFrequency: "daily", priority: 0.7 },
    { url: `${SITE_URL}/about`, lastModified: now, changeFrequency: "monthly", priority: 0.5 },
    ...tags.map((t) => ({
      url: `${SITE_URL}/c/${t.slug}`,
      lastModified: now,
      changeFrequency: "daily" as const,
      priority: 0.6,
    })),
    ...submissions.map((s) => ({
      url: `${SITE_URL}/post/${s.id}`,
      lastModified: new Date(s.submitted_at),
      changeFrequency: "weekly" as const,
      priority: 0.5,
    })),
  ];
}
