"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";

interface NavGroup {
  title: string;
  items: { href: string; label: string }[];
}

const NAV: NavGroup[] = [
  {
    title: "Get started",
    items: [
      { href: "/app", label: "Overview" },
      { href: "/app/why", label: "Why ClauDepot" },
      { href: "/app/install", label: "Install" },
    ],
  },
  {
    title: "Features",
    items: [
      { href: "/app/features", label: "All features" },
      { href: "/app/features/accounts", label: "Accounts" },
      { href: "/app/features/activities", label: "Activities" },
      { href: "/app/features/projects", label: "Projects" },
      { href: "/app/features/keys", label: "Keys" },
      { href: "/app/features/third-parties", label: "Third-parties" },
      { href: "/app/features/automations", label: "Automations" },
      { href: "/app/features/global", label: "Global" },
      { href: "/app/features/settings", label: "Settings" },
    ],
  },
  {
    title: "Reference",
    items: [
      { href: "/app/changelog", label: "Changelog" },
      { href: "/app/download", label: "Download" },
    ],
  },
];

export function DocsSidebar() {
  const pathname = usePathname() ?? "";
  return (
    <aside className="docs-sidebar" aria-label="Documentation navigation">
      <nav>
        {NAV.map((group) => (
          <section key={group.title} className="docs-nav-group">
            <h2 className="docs-nav-group-title">{group.title}</h2>
            <ul className="docs-nav-list">
              {group.items.map((item) => {
                const isActive =
                  item.href === "/app"
                    ? pathname === "/app"
                    : pathname === item.href ||
                      pathname.startsWith(item.href + "/");
                return (
                  <li key={item.href}>
                    <Link
                      href={item.href}
                      className={isActive ? "docs-nav-link is-active" : "docs-nav-link"}
                      aria-current={isActive ? "page" : undefined}
                    >
                      {item.label}
                    </Link>
                  </li>
                );
              })}
            </ul>
          </section>
        ))}
      </nav>
    </aside>
  );
}
