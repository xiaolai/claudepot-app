import { DocsSidebar } from "@/components/docs/DocsSidebar";

export default function AppDocsLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="docs-shell">
      <DocsSidebar />
      <article className="docs-main">{children}</article>
    </div>
  );
}
