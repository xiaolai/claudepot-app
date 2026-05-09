/**
 * Pure validation primitives for avatar uploads.
 *
 * Lives in its own module — no DB / blob imports — so unit tests can
 * exercise the validation surface without booting @/db/client (which
 * throws at module load if DATABASE_URL is unset). Same pattern used
 * by lib/editorial-writes/schemas.ts and the bots' lib/bots/schemas.ts.
 *
 * lib/avatars.ts re-exports everything here, so call sites that need
 * both validation and persistence can keep their single import.
 */

/** 2 MB. Most avatar pickers crop to 256×256 long before this; anyone
 *  hitting the cap is uploading something they shouldn't. */
export const MAX_AVATAR_BYTES = 2 * 1024 * 1024;

/** Allowed content types. SVG is deliberately excluded for citizen
 *  uploads — SVG can carry inline `<script>` and would need full
 *  sanitization. The bots that use SVG avatars (reader-bot invaders)
 *  bypass this API and write directly via the seed scripts. */
export const ALLOWED_AVATAR_TYPES = [
  "image/png",
  "image/jpeg",
  "image/webp",
] as const;

export type AllowedAvatarType = (typeof ALLOWED_AVATAR_TYPES)[number];

export const AVATAR_EXTENSION: Record<AllowedAvatarType, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/webp": "webp",
};

/** Magic-byte signatures for the allowed types. Probed AFTER the
 *  MIME-type check to catch a mismatch (the client claimed image/png
 *  but sent a JPEG, or claimed any image type but sent a polyglot
 *  swap-out). Returns null when the input is too short or doesn't
 *  match any allowed signature. */
export function detectAvatarMagicType(
  bytes: Uint8Array,
): AllowedAvatarType | null {
  if (bytes.length < 12) return null;
  // PNG: 89 50 4E 47 0D 0A 1A 0A
  if (
    bytes[0] === 0x89 &&
    bytes[1] === 0x50 &&
    bytes[2] === 0x4e &&
    bytes[3] === 0x47
  ) {
    return "image/png";
  }
  // JPEG: FF D8 FF
  if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) {
    return "image/jpeg";
  }
  // WebP: RIFF .... WEBP
  if (
    bytes[0] === 0x52 &&
    bytes[1] === 0x49 &&
    bytes[2] === 0x46 &&
    bytes[3] === 0x46 &&
    bytes[8] === 0x57 &&
    bytes[9] === 0x45 &&
    bytes[10] === 0x42 &&
    bytes[11] === 0x50
  ) {
    return "image/webp";
  }
  return null;
}
