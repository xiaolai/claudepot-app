import type { MetadataRoute } from "next";

const SITE_URL = process.env.NEXT_PUBLIC_SITE_URL ?? "https://claudepot.com";

export default function robots(): MetadataRoute.Robots {
  // Until production cutover (phase 11c), origin/main still hosts v1; keep the
  // dev tree noindex'd. Flip to allow when DNS cuts over.
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
