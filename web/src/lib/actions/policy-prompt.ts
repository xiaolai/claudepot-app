"use server";

import { revalidatePath } from "next/cache";
import { eq, sql } from "drizzle-orm";

import { db } from "@/db/client";
import { moderationLog, moderationPrompts } from "@/db/schema";
import { requireStaffId } from "@/lib/staff";
import { clearPromptCache } from "@/lib/moderation/prompt-store";
import { POLICY_RESPONSE_JSON_SCHEMA, buildUserPrompt } from "@/lib/moderation/prompt";
import { PolicyResponseSchema, reconcileCategory } from "@/lib/moderation/schema";
import { POLICY_MODEL, type ModerationKind } from "@/lib/moderation/types";

/**
 * Server actions for /admin/policy-prompt.
 *
 *   publishPolicyPromptAction(input):
 *     Inserts a new row into moderation_prompts and atomically
 *     flips it active. Old active row keeps its history (active
 *     becomes false). Logs to moderation_log so /admin/log shows
 *     who changed the prompt and when. Clears the prompt-store
 *     cache so the next moderate() call picks up the new prompt
 *     immediately.
 *
 *   previewPolicyPromptAction(input):
 *     Takes a DRAFT system prompt (not yet saved) and runs it
 *     against a small fixture set, returning the verdict for each.
 *     Lets staff confirm a new prompt scores known cases the way
 *     they expect before activation. Costs ~$0.0005 per preview
 *     run (5 calls × ~$0.0001 each).
 *
 * Both actions are staff-only via requireStaffId().
 */

const MIN_PROMPT_CHARS = 200;
const MAX_PROMPT_CHARS = 16_000;
const MAX_VERSION_LEN = 40;
const MAX_NOTE_LEN = 500;

export interface PublishPolicyPromptInput {
  version: string;
  systemPrompt: string;
  note?: string;
}

export type PublishPolicyPromptResult =
  | { ok: true; id: string; version: string }
  | {
      ok: false;
      reason:
        | "forbidden"
        | "validation"
        | "duplicate_version"
        | "internal";
      detail?: string;
    };

export async function publishPolicyPromptAction(
  input: PublishPolicyPromptInput,
): Promise<PublishPolicyPromptResult> {
  const staffId = await requireStaffId();
  if (!staffId) return { ok: false, reason: "forbidden" };

  const version = (input.version ?? "").trim();
  const systemPrompt = (input.systemPrompt ?? "").trim();
  const note = input.note?.trim() || undefined;

  if (
    version.length === 0 ||
    version.length > MAX_VERSION_LEN ||
    !/^[\w.\-]+$/.test(version)
  ) {
    return {
      ok: false,
      reason: "validation",
      detail:
        "version must be 1–40 chars of [A-Za-z0-9_.-] (no spaces, no special chars).",
    };
  }
  if (systemPrompt.length < MIN_PROMPT_CHARS) {
    return {
      ok: false,
      reason: "validation",
      detail: `system prompt must be at least ${MIN_PROMPT_CHARS} chars (suspicious otherwise).`,
    };
  }
  if (systemPrompt.length > MAX_PROMPT_CHARS) {
    return {
      ok: false,
      reason: "validation",
      detail: `system prompt is too long (>${MAX_PROMPT_CHARS} chars).`,
    };
  }
  if (note && note.length > MAX_NOTE_LEN) {
    return {
      ok: false,
      reason: "validation",
      detail: `note is too long (>${MAX_NOTE_LEN} chars).`,
    };
  }

  let newId: string | null = null;
  try {
    await db.transaction(async (tx) => {
      // Single transaction: deactivate prior active row, insert new
      // active row, plus the audit log entry. The partial unique
      // index on (active=true) is checked at COMMIT — both writes
      // must land before the constraint is evaluated.
      await tx
        .update(moderationPrompts)
        .set({ active: false })
        .where(eq(moderationPrompts.active, true));

      const [row] = await tx
        .insert(moderationPrompts)
        .values({
          version,
          systemPrompt,
          active: true,
          createdBy: staffId,
          note: note ?? null,
        })
        .returning({ id: moderationPrompts.id });
      newId = row?.id ?? null;

      await tx.insert(moderationLog).values({
        staffId,
        action: "approve",
        targetType: null,
        targetId: null,
        note: `policy_prompt_v=${version}`.slice(0, 500),
      });
    });
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    // Postgres unique-violation on `version` → user-facing duplicate.
    if (
      typeof msg === "string" &&
      (msg.includes("moderation_prompts_version_unique") ||
        msg.includes("23505"))
    ) {
      return { ok: false, reason: "duplicate_version" };
    }
    console.warn(`[policy-prompt] publish failed: ${msg}`);
    return { ok: false, reason: "internal", detail: msg.slice(0, 200) };
  }

  if (!newId) {
    return { ok: false, reason: "internal", detail: "insert returned no id" };
  }

  // Clear the in-process prompt cache so the next moderate() call
  // sees the new prompt without waiting for the TTL.
  clearPromptCache();

  // Revalidate paths whose rendering depends on the active prompt.
  revalidatePath("/admin/console/policy");
  revalidatePath("/admin/log");

  return { ok: true, id: newId, version };
}

/* ── Preview action ─────────────────────────────────────────── */

export interface PreviewPolicyPromptInput {
  systemPrompt: string;
}

export interface PreviewFixtureResult {
  label: string;
  kind: ModerationKind;
  expected: string;
  actual: {
    verdict: "pass" | "reject";
    category: string | null;
    confidence: "high" | "low";
    oneLineWhy: string;
  } | null;
  matched: boolean;
  elapsedMs: number;
  error?: string;
}

export type PreviewPolicyPromptResult =
  | { ok: true; results: PreviewFixtureResult[] }
  | { ok: false; reason: "forbidden" | "validation" | "no_api_key"; detail?: string };

const PREVIEW_FIXTURES: Array<{
  label: string;
  kind: ModerationKind;
  title: string;
  body: string;
  expected: string;
}> = [
  {
    label: "good submission",
    kind: "submission",
    title: "Tutorial: building an eval harness for legal-doc summarization",
    body: "Step 1: gather 50 documents with hand-written summaries. Step 2: define your judge prompt. Step 3: run pairwise comparison. We hit 87% agreement with the expert reviewer over 4 weeks.",
    expected: "pass",
  },
  {
    label: "spam submission",
    kind: "submission",
    title: "AMAZING DEAL — buy followers cheap",
    body: "Get 10K followers for $5! Visit www.spammy.example/promo. No questions asked.",
    expected: "reject:spam",
  },
  {
    label: "doxxing comment",
    kind: "comment",
    title: "",
    body: "I know who you are. Your real name is Jane Doe, you live at 1234 Maple St, Apt 4B, Springfield, your phone is 555-0142.",
    expected: "reject:doxxing",
  },
  {
    label: "security research (FP risk)",
    kind: "submission",
    title: "Reverse-engineering the EvilCorp ransomware payload",
    body: "We disassembled the EvilCorp.bin sample. The payload uses RSA-2048 with a hardcoded public key and AES-256-CBC. Mitigation: snapshot Volume Shadow Copies before the kill switch fires.",
    expected: "pass",
  },
];

export async function previewPolicyPromptAction(
  input: PreviewPolicyPromptInput,
): Promise<PreviewPolicyPromptResult> {
  const staffId = await requireStaffId();
  if (!staffId) return { ok: false, reason: "forbidden" };

  const systemPrompt = (input.systemPrompt ?? "").trim();
  if (
    systemPrompt.length < MIN_PROMPT_CHARS ||
    systemPrompt.length > MAX_PROMPT_CHARS
  ) {
    return {
      ok: false,
      reason: "validation",
      detail: `system prompt must be ${MIN_PROMPT_CHARS}–${MAX_PROMPT_CHARS} chars.`,
    };
  }

  if (!process.env.OPENAI_API_KEY) {
    return { ok: false, reason: "no_api_key" };
  }

  // Lazy-import the SDK; the action is staff-only and called from
  // server-side rendering, so the cost of the import on every
  // preview request is fine — and keeping it out of the module
  // top-level avoids the SDK paying init cost on every page load.
  const OpenAI = (await import("openai")).default;
  const client = new OpenAI({ apiKey: process.env.OPENAI_API_KEY });

  const results: PreviewFixtureResult[] = [];
  for (const fx of PREVIEW_FIXTURES) {
    const t0 = Date.now();
    try {
      const completion = await client.chat.completions.create({
        model: POLICY_MODEL,
        messages: [
          { role: "system", content: systemPrompt },
          {
            role: "user",
            content: buildUserPrompt({ kind: fx.kind, title: fx.title, body: fx.body }),
          },
        ],
        response_format: {
          type: "json_schema",
          json_schema: POLICY_RESPONSE_JSON_SCHEMA,
        },
        temperature: 0,
      });

      const choice = completion.choices[0];
      if (choice?.message?.refusal) {
        throw new Error(
          `model refused: ${String(choice.message.refusal).slice(0, 120)}`,
        );
      }
      const raw = choice?.message?.content ?? "";
      if (!raw) throw new Error("empty model response");
      const parsed = reconcileCategory(PolicyResponseSchema.parse(JSON.parse(raw)));
      const actualLabel =
        parsed.verdict === "reject"
          ? `reject:${parsed.category ?? "unknown"}`
          : "pass";

      results.push({
        label: fx.label,
        kind: fx.kind,
        expected: fx.expected,
        actual: {
          verdict: parsed.verdict,
          category: parsed.category,
          confidence: parsed.confidence,
          oneLineWhy: parsed.one_line_why,
        },
        matched: actualLabel === fx.expected,
        elapsedMs: Date.now() - t0,
      });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      results.push({
        label: fx.label,
        kind: fx.kind,
        expected: fx.expected,
        actual: null,
        matched: false,
        elapsedMs: Date.now() - t0,
        error: msg.slice(0, 200),
      });
    }
  }

  return { ok: true, results };
}

/* ── Form-data shim for the editor's <form action={...}> ───── */

export type PolicyPromptActionState = {
  ok: boolean;
  message: string;
  newVersion?: string;
};

export async function publishPolicyPromptFormAction(
  _prev: PolicyPromptActionState,
  formData: FormData,
): Promise<PolicyPromptActionState> {
  const result = await publishPolicyPromptAction({
    version: String(formData.get("version") ?? ""),
    systemPrompt: String(formData.get("systemPrompt") ?? ""),
    note: formData.get("note") ? String(formData.get("note")) : undefined,
  });
  if (result.ok) {
    return {
      ok: true,
      message: `Activated version ${result.version}.`,
      newVersion: result.version,
    };
  }
  switch (result.reason) {
    case "forbidden":
      return { ok: false, message: "Not authorized." };
    case "duplicate_version":
      return { ok: false, message: "That version label already exists." };
    case "validation":
      return { ok: false, message: result.detail ?? "Invalid input." };
    case "internal":
      return {
        ok: false,
        message: `Internal error: ${result.detail ?? "unknown"}`,
      };
  }
}

/* ── Re-export for the page server-component to read ──────── */

export async function loadPolicyPromptHistory(): Promise<
  Array<{
    id: string;
    version: string;
    active: boolean;
    note: string | null;
    systemPrompt: string;
    createdAt: Date;
    createdByUsername: string | null;
  }>
> {
  const rows = await db.execute<{
    id: string;
    version: string;
    active: boolean;
    note: string | null;
    system_prompt: string;
    created_at: Date;
    created_by_username: string | null;
  }>(sql`
    SELECT mp.id, mp.version, mp.active, mp.note,
           mp.system_prompt, mp.created_at,
           u.username AS created_by_username
      FROM moderation_prompts mp
      LEFT JOIN users u ON u.id = mp.created_by
     ORDER BY mp.created_at DESC
     LIMIT 50
  `);
  type RawRow = {
    id: string;
    version: string;
    active: boolean;
    note: string | null;
    system_prompt: string;
    created_at: Date;
    created_by_username: string | null;
  };
  const list =
    (rows as unknown as { rows?: RawRow[] }).rows ?? (rows as unknown as RawRow[]);
  return list.map((r) => ({
    id: r.id,
    version: r.version,
    active: r.active,
    note: r.note,
    systemPrompt: r.system_prompt,
    createdAt: r.created_at,
    createdByUsername: r.created_by_username,
  }));
}
