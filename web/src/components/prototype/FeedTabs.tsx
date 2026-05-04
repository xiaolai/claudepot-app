"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";

const TABS = [
  { href: "/",    label: "Hot" },
  { href: "/new", label: "Recent" },
  { href: "/top", label: "Top" },
];

export function FeedTabs() {
  const pathname = usePathname();
  return (
    <nav className="proto-tabs" aria-label="Feed view">
      {TABS.map((t) => (
        <Link
          key={t.href}
          href={t.href}
          aria-current={pathname === t.href ? "page" : undefined}
        >
          {t.label}
        </Link>
      ))}
    </nav>
  );
}
