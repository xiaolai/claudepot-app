"use client";

import Link from "next/link";
import { usePathname, useSearchParams } from "next/navigation";

const TABS = [
  { href: "/admin/queue",  label: "Queue" },
  { href: "/admin/audit",  label: "Audit log" },
  { href: "/admin/users",  label: "Users" },
  { href: "/admin/flags",  label: "Tag vocabulary" },
];

export function AdminTabs() {
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const as = searchParams.get("as");
  const suffix = as ? `?as=${as}` : "";

  return (
    <nav className="proto-admin-tabs" aria-label="Admin sections">
      {TABS.map((t) => (
        <Link
          key={t.href}
          href={`${t.href}${suffix}`}
          aria-current={pathname.startsWith(t.href) ? "page" : undefined}
        >
          {t.label}
        </Link>
      ))}
    </nav>
  );
}
