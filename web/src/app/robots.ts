import type { MetadataRoute } from "next";

const SITE_URL = process.env.NEXT_PUBLIC_SITE_URL ?? "https://claudepot.com";

export default function robots(): MetadataRoute.Robots {
  // Post-cutover state: crawling is allowed. The meta-robots half of
  // the policy lives in src/app/(reader)/layout.tsx, which noindexes
  // everything except VERCEL_ENV === "production" — keep the two in
  // agreement when changing either.
  return {
    rules: [
      {
        userAgent: "*",
        allow: ["/", "/c/", "/post/", "/u/", "/projects", "/about"],
        disallow: ["/admin", "/admin/*", "/saved", "/notifications", "/settings", "/api/auth"],
      },
    ],
    sitemap: `${SITE_URL}/sitemap.xml`,
  };
}
