/**
 * Server actions for /settings/bots — citizen-bot lifecycle from the
 * UI side. The actual business logic lives in lib/citizen-bots/;
 * these wrappers handle session auth + form parsing + cache
 * invalidation.
 */

"use server";

import { revalidatePath } from "next/cache";

import { auth } from "@/lib/auth";
import {
  createCitizenBot,
  deleteCitizenBot,
  mintTokenForBot,
} from "@/lib/citizen-bots";
import {
  createCitizenBotSchema,
  mintCitizenBotTokenSchema,
} from "@/lib/citizen-bots/schemas";

export type CreateBotFormState =
  | { phase: "idle" }
  | { phase: "ok"; botId: string; username: string }
  | { phase: "error"; message: string };

export async function createBotFormAction(
  _prev: CreateBotFormState,
  formData: FormData,
): Promise<CreateBotFormState> {
  const session = await auth();
  if (!session?.user?.id) {
    return { phase: "error", message: "Sign in to create a bot." };
  }

  const parsed = createCitizenBotSchema.safeParse({
    baseUsername: String(formData.get("baseUsername") ?? "").trim().toLowerCase(),
    displayName: optionalString(formData.get("displayName")),
    bio: optionalString(formData.get("bio")),
  });
  if (!parsed.success) {
    return {
      phase: "error",
      message: parsed.error.issues[0]?.message ?? "Validation failed.",
    };
  }

  const result = await createCitizenBot(session.user.id, parsed.data);
  if (!result.ok) {
    return {
      phase: "error",
      message: result.detail ?? `Could not create bot: ${result.reason}`,
    };
  }

  revalidatePath("/settings/bots");
  return { phase: "ok", botId: result.bot.id, username: result.bot.username };
}

export type MintBotTokenFormState =
  | { phase: "idle" }
  | {
      phase: "ok";
      plaintext: string;
      displayPrefix: string;
      granted: string[];
      dropped: string[];
    }
  | { phase: "error"; message: string };

export async function mintBotTokenFormAction(
  _prev: MintBotTokenFormState,
  formData: FormData,
): Promise<MintBotTokenFormState> {
  const session = await auth();
  if (!session?.user?.id) {
    return { phase: "error", message: "Sign in first." };
  }
  const botId = String(formData.get("botId") ?? "");
  if (!botId) {
    return { phase: "error", message: "Missing botId." };
  }
  const parsed = mintCitizenBotTokenSchema.safeParse({
    name: String(formData.get("name") ?? "").trim(),
    scopes: formData.getAll("scopes").map((v) => String(v)),
  });
  if (!parsed.success) {
    return {
      phase: "error",
      message: parsed.error.issues[0]?.message ?? "Validation failed.",
    };
  }

  const result = await mintTokenForBot(session.user.id, botId, parsed.data);
  if (!result.ok) {
    if (result.reason === "no_valid_scopes") {
      return {
        phase: "error",
        message:
          "None of the requested scopes are allowed for citizen bots. " +
          "See web/dev-docs/citizen-bots.md for the allowlist.",
      };
    }
    return { phase: "error", message: `Could not mint token: ${result.reason}` };
  }

  revalidatePath("/settings/bots");
  return {
    phase: "ok",
    plaintext: result.plaintext,
    displayPrefix: result.displayPrefix,
    granted: [...result.grantedScopes],
    dropped: [...result.droppedScopes],
  };
}

export async function deleteBotAction(
  botId: string,
): Promise<{ ok: true } | { ok: false; reason: "unauth" | "not_owner" | "not_found" }> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };
  const result = await deleteCitizenBot(session.user.id, botId);
  if (!result.ok) return result;
  revalidatePath("/settings/bots");
  return { ok: true };
}

function optionalString(v: FormDataEntryValue | null): string | undefined {
  if (v == null) return undefined;
  const s = String(v).trim();
  return s.length === 0 ? undefined : s;
}
