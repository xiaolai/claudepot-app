import type { ReactNode } from "react";

/**
 * Shell for /admin/* — the inbox at /admin and the console cluster
 * at /admin/console/*. The previous tabbed nav (AdminTabs) was
 * removed in the admin-redesign; nav lives in the inbox header
 * (chips) and the console index (cards) instead.
 *
 * This layout intentionally has no chrome of its own beyond the
 * page-frame container, so child pages own their own headers.
 */
export default function AdminLayout({ children }: { children: ReactNode }) {
  return <div className="proto-admin">{children}</div>;
}
