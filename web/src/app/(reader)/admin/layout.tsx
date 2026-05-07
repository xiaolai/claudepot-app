import Link from "next/link";
import type { ReactNode } from "react";

import { auth } from "@/lib/auth";

/**
 * Shell for /admin/* — the inbox at /admin and the console cluster
 * at /admin/console/*. The previous tabbed nav (AdminTabs) was
 * removed in the admin-redesign; nav lives in the inbox header
 * (chips) and the console index (cards) instead.
 *
 * Defense-in-depth gate: every leaf page also calls `staffGate(sp)`
 * (the rich version with dev-`?as=` support). This layout enforces a
 * hard real-session staff/system check so a new admin page added
 * without that per-page call cannot ship publicly. Layouts in
 * Next.js 15 cannot read `searchParams`, so the dev-shim path stays
 * at the page level — production is unaffected because the shim is
 * disabled in production by contract.
 */
export default async function AdminLayout({
  children,
}: {
  children: ReactNode;
}) {
  const session = await auth();
  const role = session?.user?.role;
  const isStaff = role === "staff" || role === "system";
  if (!isStaff && process.env.NODE_ENV === "production") {
    return (
      <div className="proto-admin">
        <div className="proto-admin-gate">
          <p className="proto-empty proto-empty-spaced">
            Staff only. <Link href="/login">Sign in</Link> to continue.
          </p>
        </div>
      </div>
    );
  }
  return <div className="proto-admin">{children}</div>;
}
