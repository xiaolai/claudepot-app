import { Suspense } from "react";
import { AdminTabs } from "@/components/prototype/AdminTabs";

export default function AdminLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="proto-admin">
      <header className="proto-admin-header">
        <h1 className="proto-admin-title">Admin</h1>
        <p className="proto-admin-dek">
          AI moderation oversight, audit log, users, and tag vocabulary.
        </p>
      </header>
      <Suspense fallback={<nav className="proto-admin-tabs" aria-label="Admin sections" />}>
        <AdminTabs />
      </Suspense>
      <div className="proto-admin-body">{children}</div>
    </div>
  );
}
