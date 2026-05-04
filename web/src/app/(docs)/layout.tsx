import type { Metadata } from "next";
import Link from "next/link";
import { JetBrains_Mono } from "next/font/google";
import { Analytics } from "@vercel/analytics/next";
import { DocsSidebar } from "@/components/docs/DocsSidebar";
import "@/styles/theme.css";
import "@/styles/font-mono-only.css";
import "@/styles/prototype.css";
import "@/styles/docs.css";

const jetbrainsMono = JetBrains_Mono({
  subsets: ["latin"],
  display: "swap",
  variable: "--font-mono",
});

const SITE_DESCRIPTION =
  "ClauDepot — a control panel for Claude Code and Claude Desktop.";

export const metadata: Metadata = {
  title: {
    default: "ClauDepot",
    template: "%s · ClauDepot",
  },
  description: SITE_DESCRIPTION,
  applicationName: "ClauDepot",
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

export default function DocsRootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" className={jetbrainsMono.variable}>
      <body>
        <header className="docs-topbar">
          <Link href="/app" className="docs-brand" aria-label="ClauDepot">
            ClauDepot
          </Link>
          <nav className="docs-topbar-nav">
            <Link href="/app/why">Why</Link>
            <Link href="/app/install">Install</Link>
            <Link href="/app/features">Features</Link>
            <Link href="/app/changelog">Changelog</Link>
            <Link href="/" className="docs-topbar-reader">Reader →</Link>
          </nav>
        </header>
        <div className="docs-shell">
          <DocsSidebar />
          <main className="docs-main">{children}</main>
        </div>
        <footer className="docs-footer">
          <p>
            ClauDepot is built to the{" "}
            <Link href="https://sha.nnon.ai">software-robot definition</Link>.
          </p>
        </footer>
        <Analytics />
      </body>
    </html>
  );
}
