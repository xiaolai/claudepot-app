import Link from "next/link";
import {
  Bell,
  Bookmark,
  ChevronUp,
  CircleDashed,
  KeyRound,
  Settings as SettingsIcon,
  User,
} from "lucide-react";

export type AccountSidebarPage =
  | "profile"
  | "notifications"
  | "saved"
  | "upvoted"
  | "pending"
  | "settings"
  | "tokens";

/** Shared left-rail navigation for the personal-hub pages
 *  (`/u/<your-handle>`, `/notifications`, `/saved`, `/upvoted`,
 *  `/pending`, `/settings`).
 *
 *  The first link doubles as the section title — clicking the
 *  @handle goes to the profile, matching the OfficeSidebar pattern
 *  where the first list item is also the area's landing page.
 *
 *  asParam preserves the dev `?as=<username>` shim across navigation
 *  when the page entered via that shim. Pages on real Auth.js
 *  sessions should leave it undefined. */
export function AccountSidebar({
  current,
  username,
  asParam,
}: {
  current: AccountSidebarPage;
  username: string;
  asParam?: string | null;
}) {
  const aria = (key: AccountSidebarPage) =>
    key === current ? ("page" as const) : undefined;
  const suffix = asParam ? `?as=${asParam}` : "";
  return (
    <nav className="proto-page-aside-nav" aria-label="Your account">
      <ul>
        <li>
          <Link
            href={`/u/${username}${suffix}`}
            aria-current={aria("profile")}
          >
            <User size={14} aria-hidden /> @{username}
          </Link>
        </li>
        <li>
          <Link
            href={`/notifications${suffix}`}
            aria-current={aria("notifications")}
          >
            <Bell size={14} aria-hidden /> Notifications
          </Link>
        </li>
        <li>
          <Link
            href={`/saved${suffix}`}
            aria-current={aria("saved")}
          >
            <Bookmark size={14} aria-hidden /> Saved
          </Link>
        </li>
        <li>
          <Link
            href={`/upvoted${suffix}`}
            aria-current={aria("upvoted")}
          >
            <ChevronUp size={14} aria-hidden /> Upvoted
          </Link>
        </li>
        <li>
          <Link
            href={`/pending${suffix}`}
            aria-current={aria("pending")}
          >
            <CircleDashed size={14} aria-hidden /> Pending
          </Link>
        </li>
        <li>
          <Link
            href={`/settings${suffix}`}
            aria-current={aria("settings")}
          >
            <SettingsIcon size={14} aria-hidden /> Settings
          </Link>
        </li>
        <li>
          <Link
            href={`/settings/tokens${suffix}`}
            aria-current={aria("tokens")}
          >
            <KeyRound size={14} aria-hidden /> API tokens
          </Link>
        </li>
      </ul>
    </nav>
  );
}
