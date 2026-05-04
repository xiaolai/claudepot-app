import type { Metadata } from "next";
import { Suspense } from "react";
import { JetBrains_Mono } from "next/font/google";
import { Analytics } from "@vercel/analytics/next";
import { and, count, eq, isNull } from "drizzle-orm";
import { PrototypeNav } from "@/components/prototype/PrototypeNav";
import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { notifications } from "@/db/schema";
import "@/styles/theme.css";
import "@/styles/font-mono-only.css";
import "@/styles/prototype.css";

// Single typeface for the whole site. The three legacy --font-*
// variables (display / serif-body / body) are aliased to this one
// in font-mono-only.css, so existing class rules keep working
// unchanged.
const jetbrainsMono = JetBrains_Mono({
  subsets: ["latin"],
  display: "swap",
  variable: "--font-mono",
});

const SITE_DESCRIPTION =
  "A daily reader for builders working with AI tools.";

export const metadata: Metadata = {
  title: {
    default: "ClauDepot",
    template: "%s · SHANNON",
  },
  description: SITE_DESCRIPTION,
  applicationName: "ClauDepot",
  robots: { index: false, follow: false },
  openGraph: {
    type: "website",
    siteName: "ClauDepot",
    title: "ClauDepot",
    description: SITE_DESCRIPTION,
  },
  twitter: {
    card: "summary",
    title: "ClauDepot",
    description: SITE_DESCRIPTION,
  },
};

export default async function PrototypeLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  // Pull session.user.image once at layout level so the nav can render
  // the real OAuth photo (path A). Falls back to identicon when null.
  // sessionUsername is the DB username (slug), not the OAuth display
  // name — that's what /u/<username> profile URLs need.
  const session = await auth();
  const sessionImageUrl = session?.user?.image ?? null;
  const sessionUsername = session?.user?.username ?? null;
  const sessionIsStaff =
    session?.user?.role === "staff" || session?.user?.role === "system";

  // Real unread count for the nav badge. Computed here in the layout
  // (server component) because PrototypeNav is "use client" and can't
  // hit the DB. Only resolvable for real Auth.js sessions — App Router
  // layouts cannot read searchParams (Next caching constraint), so the
  // dev `?as=<username>` shim path falls through to 0. That's honest:
  // better an under-count than the previous mock that always said 1.
  let unreadNotifications = 0;
  if (session?.user?.id) {
    try {
      const [row] = await db
        .select({ n: count() })
        .from(notifications)
        .where(
          and(
            eq(notifications.userId, session.user.id),
            isNull(notifications.readAt),
          ),
        );
      unreadNotifications = row?.n ?? 0;
    } catch (err) {
      // Layout MUST render; transient DB blips show as zero, not 500.
      console.error("[layout] unread count query failed:", err);
    }
  }

  return (
    <html lang="en" className={jetbrainsMono.variable}>
      <body>
        <Suspense fallback={<nav className="proto-nav" aria-label="Main" />}>
          <PrototypeNav
            sessionImageUrl={sessionImageUrl}
            sessionUsername={sessionUsername}
            sessionIsStaff={sessionIsStaff}
            unreadNotifications={unreadNotifications}
          />
        </Suspense>
        <main>{children}</main>
        {/*
         * Vercel Web Analytics replaces the prior Cloudflare beacon.
         * CF Web Analytics requires the host to be proxied through CF
         * (orange cloud), but sha.com is on Vercel direct (gray
         * cloud DNS-only), so the CF beacon's POST to /cdn-cgi/rum
         * was CORS-blocked and recorded zero events. Vercel Analytics
         * runs natively on this stack with no CORS surface.
         */}
        <Analytics />
      </body>
    </html>
  );
}
