import type { MetadataRoute } from "next";

import { getSitemapSubmissions, getAllTags } from "@/db/queries";

const SITE_URL = process.env.NEXT_PUBLIC_SITE_URL ?? "https://claudepot.com";

// Static product-docs routes under /app — mirrors the on-disk
// src/app/(reader)/app tree. Keep in sync when adding a docs page
// (DocsSidebar renders the same list, minus features/memory which is
// intentionally unlisted in the sidebar but still a public page).
const APP_DOCS_PATHS = [
  "/app",
  "/app/why",
  "/app/install",
  "/app/features",
  "/app/features/accounts",
  "/app/features/activities",
  "/app/features/automations",
  "/app/features/global",
  "/app/features/keys",
  "/app/features/memory",
  "/app/features/projects",
  "/app/features/settings",
  "/app/features/third-parties",
  "/app/changelog",
  "/app/download",
] as const;

// Docs pages that change with releases vs. evergreen feature pages.
const APP_DOCS_WEEKLY = new Set(["/app", "/app/changelog", "/app/download"]);

export default async function sitemap(): Promise<MetadataRoute.Sitemap> {
  const [submissions, tags] = await Promise.all([
    getSitemapSubmissions(),
    getAllTags(),
  ]);
  const now = new Date();
  return [
    { url: `${SITE_URL}/`, lastModified: now, changeFrequency: "hourly", priority: 1.0 },
    { url: `${SITE_URL}/new`, lastModified: now, changeFrequency: "hourly", priority: 0.9 },
    { url: `${SITE_URL}/top`, lastModified: now, changeFrequency: "daily", priority: 0.8 },
    { url: `${SITE_URL}/c`, lastModified: now, changeFrequency: "daily", priority: 0.7 },
    { url: `${SITE_URL}/links`, lastModified: now, changeFrequency: "daily", priority: 0.6 },
    { url: `${SITE_URL}/about`, lastModified: now, changeFrequency: "monthly", priority: 0.5 },
    { url: `${SITE_URL}/privacy`, lastModified: now, changeFrequency: "yearly", priority: 0.3 },
    { url: `${SITE_URL}/terms`, lastModified: now, changeFrequency: "yearly", priority: 0.3 },
    ...APP_DOCS_PATHS.map((p) => ({
      url: `${SITE_URL}${p}`,
      lastModified: now,
      changeFrequency: APP_DOCS_WEEKLY.has(p)
        ? ("weekly" as const)
        : ("monthly" as const),
      priority: p === "/app" ? 0.8 : APP_DOCS_WEEKLY.has(p) ? 0.7 : 0.6,
    })),
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
