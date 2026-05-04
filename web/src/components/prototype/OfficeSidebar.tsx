import Link from "next/link";
import { ScrollText, Volume2, BookOpen } from "lucide-react";

export type OfficeSidebarPage = "office" | "transparency" | "voice" | "rubric";

/** Shared left-rail navigation for the four /office area pages.
 *  Pass the current page key so the active link gets aria-current="page"
 *  (which the .proto-page-aside-nav stylesheet picks up for the accent
 *  border + accent-ink color). */
export function OfficeSidebar({ current }: { current: OfficeSidebarPage }) {
  const ariaCurrent = (key: OfficeSidebarPage) =>
    key === current ? ("page" as const) : undefined;
  return (
    <nav className="proto-page-aside-nav" aria-label="The office">
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
      </ul>
    </nav>
  );
}
