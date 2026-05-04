"use client";

import { useEffect, useRef } from "react";
import Link from "next/link";
import { usePathname, useSearchParams } from "next/navigation";
import { Bell, Bookmark, LogOut, Settings, User } from "lucide-react";
import { Logo } from "./Logo";
import { UserAvatar } from "./Avatar";
import { signOutAction } from "@/lib/actions/auth";

const NAV_ITEMS = [
  { href: "/",          label: "Home",     match: (p: string) => p === "/" || p === "/new" || p === "/top" },
  { href: "/c",        label: "Tags",     match: (p: string) => p.startsWith("/c") },
  { href: "/office",   label: "Office",   match: (p: string) => p.startsWith("/office") },
  { href: "/about",    label: "About",    match: (p: string) => p.startsWith("/about") },
];

// Local fallback for the dev `?as=` shim only. Real-session staff
// status comes from the DB role enum via the layout (sessionIsStaff
// prop). This list is the prototype's hand-wave for shim-mode users
// in dev — never consulted in production.
const DEV_SHIM_STAFF = new Set(["ada", "lixiaolai"]);

function withAuth(href: string, as: string | null): string {
  if (!as) return href;
  const sep = href.includes("?") ? "&" : "?";
  return `${href}${sep}as=${as}`;
}

interface NavProps {
  /** Real session image URL (Auth.js session.user.image). If present we
   * render it; otherwise we fall back to the identicon for the username
   * from the dev-shim or session. */
  sessionImageUrl?: string | null;
  /** DB username (slug) for the signed-in user. */
  sessionUsername?: string | null;
  /** Server-derived staff flag from session.user.role. Authoritative
   * for the real-auth path; the dev `?as=` shim falls back to the
   * `DEV_SHIM_STAFF` allowlist below. */
  sessionIsStaff?: boolean;
  /** Real unread notifications count from the DB, computed in the layout.
   * Only meaningful for real Auth.js sessions; dev-shim users see 0
   * because layouts can't read searchParams in App Router. */
  unreadNotifications?: number;
}

export function PrototypeNav({
  sessionImageUrl = null,
  sessionUsername = null,
  sessionIsStaff = false,
  unreadNotifications = 0,
}: NavProps) {
  const pathname = usePathname();
  const searchParams = useSearchParams();
  // The `?as=<username>` shim is dev-only — a public URL must not be
  // able to make any anonymous visitor's nav say `@somebody`. The real
  // session is always authoritative; the URL fallback only fires in
  // non-production. process.env.NODE_ENV is inlined at build time, so
  // this read works in a client component.
  const fallbackAs =
    process.env.NODE_ENV === "production" ? null : searchParams.get("as");
  const as = sessionUsername ?? fallbackAs;
  // Real session: trust sessionIsStaff. Shim path (dev-only): allowlist.
  const isStaff = sessionUsername
    ? sessionIsStaff
    : as
      ? DEV_SHIM_STAFF.has(as)
      : false;
  // Real unread count from the DB, computed by the layout. The previous
  // implementation was a username-derived mock that always read "1" for
  // anyone whose name started with a character with charCode % 5 == 0.
  const unread = unreadNotifications;

  // Close the account dropdown on route change so a click on a menu
  // item doesn't leave the panel hanging open over the new page.
  // <details> open state is DOM-resident, not React state, so we
  // imperatively reset it when the path changes.
  const accountMenuRef = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    if (accountMenuRef.current) accountMenuRef.current.open = false;
  }, [pathname]);

  return (
    <nav className="proto-nav" aria-label="Main">
      <Link href={withAuth("/", as)} className="proto-nav-brand">
        <Logo size={36} className="proto-nav-logo" />
        <span className="proto-nav-wordmark">SHANNON</span>
      </Link>
      {NAV_ITEMS.map((item) => (
        <Link
          key={item.href}
          href={withAuth(item.href, as)}
          aria-current={item.match(pathname) ? "page" : undefined}
        >
          {item.label}
        </Link>
      ))}
      {isStaff && (
        <Link
          href={withAuth("/admin", as)}
          className="proto-nav-staff"
          aria-current={pathname.startsWith("/admin") ? "page" : undefined}
        >
          Admin
        </Link>
      )}
      <Link href={withAuth("/submit", as)} className="proto-nav-cta">
        Submit
      </Link>
      {as ? (
        <details className="proto-nav-menu" ref={accountMenuRef}>
          <summary
            className="proto-nav-avatar"
            title={`Signed in as @${as}`}
            aria-label={
              unread > 0
                ? `Account menu — @${as}, ${unread} unread notifications`
                : `Account menu — @${as}`
            }
          >
            <UserAvatar username={as} imageUrl={sessionImageUrl} size={20} />
            <span>@{as}</span>
            {unread > 0 && (
              <span className="proto-nav-unread-dot" aria-hidden />
            )}
          </summary>
          <div className="proto-nav-menu-panel" role="menu">
            <Link href={withAuth(`/u/${as}`, as)} role="menuitem">
              <User size={14} aria-hidden /> Profile
            </Link>
            <Link
              href={withAuth("/notifications", as)}
              role="menuitem"
            >
              <Bell size={14} aria-hidden /> Notifications
              {unread > 0 && (
                <span className="proto-nav-menu-count">{unread}</span>
              )}
            </Link>
            <Link href={withAuth("/saved", as)} role="menuitem">
              <Bookmark size={14} aria-hidden /> Saved
            </Link>
            <Link href={withAuth("/settings", as)} role="menuitem">
              <Settings size={14} aria-hidden /> Settings
            </Link>
            <form action={signOutAction} className="proto-nav-menu-signout">
              <button type="submit" role="menuitem">
                <LogOut size={14} aria-hidden /> Sign out
              </button>
            </form>
          </div>
        </details>
      ) : (
        <Link href="/login" className="proto-nav-login">
          Sign in
        </Link>
      )}
    </nav>
  );
}
