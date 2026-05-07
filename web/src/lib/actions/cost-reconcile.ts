"use server";

import { revalidatePath } from "next/cache";
import { redirect } from "next/navigation";
import { eq } from "drizzle-orm";
import { z } from "zod";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { providerInvoices } from "@/db/schema";

/* ── Helpers ────────────────────────────────────────────────── */

const MONTH_RE = /^\d{4}-(0[1-9]|1[0-2])$/;

const upsertInput = z.object({
  provider: z
    .string()
    .trim()
    .min(1)
    .max(40)
    .regex(/^[a-z0-9_-]+$/i, "Provider must be alphanumeric, dashes, or underscores."),
  month: z
    .string()
    .regex(MONTH_RE, "Month must be in YYYY-MM form (e.g. 2026-05)."),
  invoicedUsd: z.number().nonnegative().max(1_000_000),
  notes: z.string().trim().max(500).optional(),
});

async function requireStaffSession(): Promise<{ userId: string } | null> {
  const session = await auth();
  if (!session?.user?.id) return null;
  const role = session.user.role;
  if (role !== "staff" && role !== "system") return null;
  return { userId: session.user.id };
}

/* ── Upsert action ──────────────────────────────────────────── */

/**
 * Form-posted server action. Inputs come as FormData strings; we
 * parse, validate, upsert, then redirect back so the page picks up
 * the new row. Validation failures redirect with ?error=... so
 * staff can see what's wrong without a separate alert surface.
 */
export async function upsertProviderInvoice(formData: FormData): Promise<void> {
  const staff = await requireStaffSession();
  if (!staff) {
    redirect("/admin/console/cost-reconcile?error=unauth");
  }

  const provider = String(formData.get("provider") ?? "").toLowerCase();
  const month = String(formData.get("month") ?? "");
  const usdRaw = formData.get("invoicedUsd");
  const notes = String(formData.get("notes") ?? "").trim();

  const usd = typeof usdRaw === "string" ? Number.parseFloat(usdRaw) : NaN;
  const parsed = upsertInput.safeParse({
    provider,
    month,
    invoicedUsd: usd,
    notes: notes.length === 0 ? undefined : notes,
  });
  if (!parsed.success) {
    const issue = parsed.error.issues[0]?.message ?? "validation";
    redirect(
      `/admin/console/cost-reconcile?error=${encodeURIComponent(issue)}`,
    );
  }

  // Idempotent: ON CONFLICT (provider, month) DO UPDATE so re-uploads
  // (e.g. credit applied) overwrite the previous row.
  await db
    .insert(providerInvoices)
    .values({
      provider: parsed.data.provider,
      month: parsed.data.month,
      invoicedUsd: parsed.data.invoicedUsd.toFixed(2),
      uploadedBy: staff.userId,
      notes: parsed.data.notes ?? null,
    })
    .onConflictDoUpdate({
      target: [providerInvoices.provider, providerInvoices.month],
      set: {
        invoicedUsd: parsed.data.invoicedUsd.toFixed(2),
        uploadedBy: staff.userId,
        uploadedAt: new Date(),
        notes: parsed.data.notes ?? null,
      },
    });

  revalidatePath("/admin/console/cost-reconcile");
  redirect("/admin/console/cost-reconcile?ok=1");
}

/* ── Delete action ──────────────────────────────────────────── */

export async function deleteProviderInvoice(formData: FormData): Promise<void> {
  const staff = await requireStaffSession();
  if (!staff) {
    redirect("/admin/console/cost-reconcile?error=unauth");
  }
  const id = String(formData.get("id") ?? "");
  if (!/^[0-9a-f-]{36}$/i.test(id)) {
    redirect("/admin/console/cost-reconcile?error=bad+id");
  }
  await db.delete(providerInvoices).where(eq(providerInvoices.id, id));
  revalidatePath("/admin/console/cost-reconcile");
  redirect("/admin/console/cost-reconcile?ok=deleted");
}
