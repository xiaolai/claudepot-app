/**
 * Server actions for the /settings avatar panel.
 *
 * Two surfaces consume them:
 *   - Web UI form (AvatarPanel) — calls these directly.
 *   - REST endpoint (POST/DELETE /api/v1/users/me/avatar) — calls
 *     the underlying lib/avatars.ts helpers, not these actions, so
 *     the PAT path doesn't pull in next/cache or auth().
 *
 * Same lib helpers, different transports.
 */

"use server";

import { revalidatePath } from "next/cache";

import { auth } from "@/lib/auth";
import { clearAvatar, setAvatar } from "@/lib/avatars";

export type SetAvatarFormState =
  | { phase: "idle" }
  | { phase: "ok"; avatarUrl: string }
  | { phase: "error"; message: string };

export async function setAvatarFormAction(
  _prev: SetAvatarFormState,
  formData: FormData,
): Promise<SetAvatarFormState> {
  const session = await auth();
  if (!session?.user?.id) {
    return { phase: "error", message: "Sign in to set an avatar." };
  }

  const file = formData.get("avatar");
  if (!(file instanceof File) || file.size === 0) {
    return { phase: "error", message: "Pick an image file first." };
  }

  const bytes = new Uint8Array(await file.arrayBuffer());
  const result = await setAvatar(session.user.id, bytes, file.type);
  if (!result.ok) {
    if (result.reason === "user_not_found") {
      return {
        phase: "error",
        message:
          "Account no longer exists. Sign out and sign in to refresh.",
      };
    }
    return {
      phase: "error",
      message: result.detail ?? "Avatar validation failed.",
    };
  }

  // The avatar appears in the masthead and on every author byline,
  // so blow the page-level cache rather than per-route.
  revalidatePath("/", "layout");
  return { phase: "ok", avatarUrl: result.url };
}

export async function clearAvatarAction(): Promise<
  { ok: true } | { ok: false; reason: "unauth" | "user_not_found" }
> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };
  const result = await clearAvatar(session.user.id);
  if (!result.ok) return { ok: false, reason: "user_not_found" };
  revalidatePath("/", "layout");
  return { ok: true };
}
