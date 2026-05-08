"use server";

import { redirect } from "next/navigation";
import { eq, sql } from "drizzle-orm";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { linkCategories, links } from "@/db/schema/links";

const MAX_NAME = 80;
const MAX_DESC = 200;
const MAX_URL = 500;

function kebab(s: string, maxWords = 8): string {
  return s
    .toLowerCase()
    .replace(/[—–]/g, "-")
    .replace(/[^\w\s-]/g, " ")
    .trim()
    .split(/\s+/)
    .slice(0, maxWords)
    .join("-")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
}

async function nextFreeSlug(base: string): Promise<string> {
  if (!base) base = "link";
  // Pull every existing slug that starts with `base` so we can pick the
  // smallest free numeric suffix in one trip. Cheap at this scale.
  const rows = await db
    .select({ slug: links.slug })
    .from(links)
    .where(sql`${links.slug} = ${base} OR ${links.slug} LIKE ${base + "-%"}`);
  const taken = new Set(rows.map((r) => r.slug));
  if (!taken.has(base)) return base;
  let i = 2;
  while (taken.has(`${base}-${i}`)) i += 1;
  return `${base}-${i}`;
}

function back(params: Record<string, string>): never {
  const qs = new URLSearchParams(params).toString();
  redirect(`/links/suggest?${qs}`);
}

export async function suggestLinkAction(formData: FormData): Promise<void> {
  const session = await auth();
  if (!session?.user) redirect("/login?callbackUrl=/links/suggest");

  const url = String(formData.get("url") ?? "").trim();
  const name = String(formData.get("name") ?? "").trim();
  const description = String(formData.get("description") ?? "").trim();
  const primaryCategorySlug = String(
    formData.get("primaryCategorySlug") ?? "",
  ).trim();

  // ── validation ────────────────────────────────────────────
  if (!url || !name || !primaryCategorySlug) {
    back({ error: "Please fill the required fields.", url, name, description });
  }
  if (url.length > MAX_URL || name.length > MAX_NAME || description.length > MAX_DESC) {
    back({ error: "One of the fields is too long.", url, name, description });
  }
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    back({ error: "URL is not parseable.", url, name, description });
  }
  if (!/^https?:$/.test(parsed!.protocol)) {
    back({ error: "URL must use http or https.", url, name, description });
  }

  // ── duplicate check ──────────────────────────────────────
  const existing = await db
    .select({ id: links.id, status: links.status })
    .from(links)
    .where(eq(links.url, url))
    .limit(1);
  if (existing.length > 0) {
    const status = existing[0].status;
    const msg =
      status === "pending"
        ? "This URL is already in the review queue."
        : "This URL is already in the directory.";
    back({ error: msg, url, name, description });
  }

  // ── category check ───────────────────────────────────────
  const cat = await db
    .select({ slug: linkCategories.slug })
    .from(linkCategories)
    .where(eq(linkCategories.slug, primaryCategorySlug))
    .limit(1);
  if (cat.length === 0) {
    back({
      error: "Pick a category from the list.",
      url,
      name,
      description,
    });
  }

  // ── insert ───────────────────────────────────────────────
  const slug = await nextFreeSlug(kebab(name));
  await db.insert(links).values({
    slug,
    name,
    url,
    description,
    primaryCategorySlug,
    categorySlugs: [primaryCategorySlug],
    status: "pending",
    suggestedBy: session.user.id,
  });

  redirect("/links/suggest?status=submitted");
}

/* ── Curator queue actions (staff-only) ─────────────────── */

async function requireStaff() {
  const session = await auth();
  const role = session?.user?.role;
  if (role !== "staff" && role !== "system") {
    redirect("/admin");
  }
  return session!;
}

export async function approveLinkAction(formData: FormData): Promise<void> {
  await requireStaff();
  const id = String(formData.get("id") ?? "").trim();
  if (!id) redirect("/admin/links");

  await db
    .update(links)
    .set({ status: "active", updatedAt: new Date() })
    .where(eq(links.id, id));

  // Bust the directory caches so the link surfaces immediately.
  const { revalidatePath } = await import("next/cache");
  revalidatePath("/links");
  revalidatePath("/admin/links");
  redirect("/admin/links");
}

export async function rejectLinkAction(formData: FormData): Promise<void> {
  await requireStaff();
  const id = String(formData.get("id") ?? "").trim();
  if (!id) redirect("/admin/links");

  await db
    .update(links)
    .set({ status: "archived", updatedAt: new Date() })
    .where(eq(links.id, id));

  const { revalidatePath } = await import("next/cache");
  revalidatePath("/admin/links");
  redirect("/admin/links");
}
