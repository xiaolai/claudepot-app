import { redirect } from "next/navigation";

export default async function AdminIndex({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
}) {
  const sp = await searchParams;
  redirect(sp.as ? `/admin/queue?as=${sp.as}` : "/admin/queue");
}
