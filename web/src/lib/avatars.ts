/**
 * Avatar set/clear helpers.
 *
 * Two surfaces consume these:
 *   - REST: app/api/v1/users/me/avatar/route.ts (POST + DELETE)
 *   - Server action: lib/actions/avatar.ts (called from the
 *     /settings avatar panel)
 *
 * Both write users.image AND users.avatarUrl in lockstep. The
 * resolver in lib/api/queries.ts prefers `image` over `avatarUrl`,
 * so writing both ensures the new avatar is the one rendered
 * regardless of which column a downstream consumer reads.
 *
 * Storage: Vercel Blob at `avatars/<userId>.<ext>`. Stable per user
 * — re-upload overwrites in place. cacheControlMaxAge=86400 (1 day)
 * so a profile-pic change propagates within a day; vs the immutable
 * 1y header used for bot avatars (which never change post-mint).
 */

import { put } from "@vercel/blob";
import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { users } from "@/db/schema";
import {
  ALLOWED_AVATAR_TYPES,
  AVATAR_EXTENSION,
  detectAvatarMagicType,
  MAX_AVATAR_BYTES,
  type AllowedAvatarType,
} from "@/lib/avatar-validation";

// Re-export so existing call sites that imported these from
// "@/lib/avatars" keep working without churn.
export {
  ALLOWED_AVATAR_TYPES,
  MAX_AVATAR_BYTES,
  type AllowedAvatarType,
} from "@/lib/avatar-validation";

export type SetAvatarResult =
  | { ok: true; url: string }
  | {
      ok: false;
      reason: "too_large" | "bad_type" | "magic_mismatch" | "user_not_found";
      detail?: string;
    };

export async function setAvatar(
  userId: string,
  bytes: Uint8Array,
  declaredContentType: string,
): Promise<SetAvatarResult> {
  if (bytes.length === 0) {
    return { ok: false, reason: "bad_type", detail: "Empty file." };
  }
  if (bytes.length > MAX_AVATAR_BYTES) {
    return {
      ok: false,
      reason: "too_large",
      detail: `Avatar must be ≤ ${MAX_AVATAR_BYTES} bytes. Got ${bytes.length}.`,
    };
  }
  if (!ALLOWED_AVATAR_TYPES.includes(declaredContentType as AllowedAvatarType)) {
    return {
      ok: false,
      reason: "bad_type",
      detail: `Content type must be one of: ${ALLOWED_AVATAR_TYPES.join(", ")}. Got ${declaredContentType}.`,
    };
  }
  const detected = detectAvatarMagicType(bytes);
  if (!detected) {
    return {
      ok: false,
      reason: "magic_mismatch",
      detail:
        "File header does not match a recognized image format. " +
        "Make sure the file is a real PNG, JPEG, or WebP.",
    };
  }
  if (detected !== declaredContentType) {
    return {
      ok: false,
      reason: "magic_mismatch",
      detail: `Declared content type (${declaredContentType}) does not match file header (${detected}).`,
    };
  }

  const ext = AVATAR_EXTENSION[detected];
  const path = `avatars/${userId}.${ext}`;
  // @vercel/blob's put() accepts ArrayBuffer | Buffer | File but not
  // a bare Uint8Array; wrap into a Buffer slice so we don't allocate
  // a fresh copy.
  const buf = Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const { url } = await put(path, buf, {
    access: "public",
    contentType: detected,
    addRandomSuffix: false,
    allowOverwrite: true,
    // 1 day so a profile change propagates within 24h; bot avatars
    // (which never change post-mint) use 1y from the seed scripts
    // instead. 1 day is the trade-off between cache hit rate and
    // change-propagation latency.
    cacheControlMaxAge: 60 * 60 * 24,
  });

  const updated = await db
    .update(users)
    .set({ image: url, avatarUrl: url, updatedAt: new Date() })
    .where(eq(users.id, userId))
    .returning({ id: users.id });

  if (updated.length === 0) {
    return { ok: false, reason: "user_not_found" };
  }
  return { ok: true, url };
}

export type ClearAvatarResult =
  | { ok: true }
  | { ok: false; reason: "user_not_found" };

export async function clearAvatar(
  userId: string,
): Promise<ClearAvatarResult> {
  // We do NOT delete the blob — old avatar URLs may still be in
  // cached DTOs / in flight in clients. The DB columns get cleared,
  // so resolvers fall through to "no avatar." A future cleanup job
  // can reap blobs whose path doesn't match any current user's
  // avatar URL.
  const updated = await db
    .update(users)
    .set({ image: null, avatarUrl: null, updatedAt: new Date() })
    .where(eq(users.id, userId))
    .returning({ id: users.id });

  if (updated.length === 0) {
    return { ok: false, reason: "user_not_found" };
  }
  return { ok: true };
}
