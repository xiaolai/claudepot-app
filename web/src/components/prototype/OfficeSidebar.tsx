import Link from "next/link";
import { ScrollText, Volume2, BookOpen, Shield, DollarSign } from "lucide-react";

export type OfficeSidebarPage =
  | "office"
  | "transparency"
  | "voice"
  | "rubric"
  | "policy"
  | "costs";

/** Shared left-rail navigation for the /office area pages.
 *  Pass the current page key so the active link gets aria-current="page"
 *  (which the .proto-page-aside-nav stylesheet picks up for the accent
 *  border + accent-ink color). */
export function OfficeSidebar({ current }: { current: OfficeSidebarPage }) {
  const ariaCurrent = (key: OfficeSidebarPage) =>
    key === current ? ("page" as const) : undefined;
  return (
    <nav className="proto-page-aside-nav" aria-label="The office">
      <details className="proto-toc-details">
        <summary className="proto-page-aside-nav-title">Office</summary>
        <ul>
          <li>
            <Link href="/office" aria-current={ariaCurrent("office")}>
              The Office
            </Link>
          </li>
          <li>
            <Link
              href="/office/transparency"
              aria-current={ariaCurrent("transparency")}
            >
              <ScrollText size={14} aria-hidden /> Transparency
            </Link>
          </li>
          <li>
            <Link href="/office/voice" aria-current={ariaCurrent("voice")}>
              <Volume2 size={14} aria-hidden /> Voice &amp; audience
            </Link>
          </li>
          <li>
            <Link href="/office/rubric" aria-current={ariaCurrent("rubric")}>
              <BookOpen size={14} aria-hidden /> The rubric
            </Link>
          </li>
          <li>
            <Link href="/office/policy" aria-current={ariaCurrent("policy")}>
              <Shield size={14} aria-hidden /> Policy moderation
            </Link>
          </li>
          <li>
            <Link href="/office/costs" aria-current={ariaCurrent("costs")}>
              <DollarSign size={14} aria-hidden /> Costs
            </Link>
          </li>
        </ul>
      </details>
    </nav>
  );
}
