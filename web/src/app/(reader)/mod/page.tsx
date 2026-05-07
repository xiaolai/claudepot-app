import { redirect } from "next/navigation";

/**
 * /mod is the operator's muscle-memory shortcut for the moderation
 * inbox. Originally aliased to /admin/queue; after the admin
 * redesign the inbox lives at /admin (Today) and /mod redirects
 * there. Keep `?as=` to preserve the dev shim across the bounce.
 */
export default async function ModRedirect({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
}) {
  const sp = await searchParams;
  redirect(sp.as ? `/admin?as=${sp.as}` : "/admin");
}
