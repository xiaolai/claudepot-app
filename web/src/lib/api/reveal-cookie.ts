/**
 * One-time encrypted cookie for revealing a freshly-minted PAT.
 *
 * Mechanism: AES-256-GCM under a key derived from AUTH_SECRET. The
 * mint form-action calls `setRevealCookie(plaintext)` then redirects
 * to /settings/tokens/reveal; the reveal page calls
 * `peekRevealCookie()` to read the plaintext (Server Component-safe,
 * no cookie write) and renders it. A Server Action triggered by the
 * "Done" button calls `deleteRevealCookie()` to enforce single-use.
 * The 120-second TTL is the backstop for users who navigate away.
 *
 * Why this beats round-tripping the secret through React form state
 * (the previous implementation): the plaintext lives in the encrypted
 * cookie for one request, then in the server-rendered HTML once,
 * never in the JS heap as a React `state.plaintext` value. DevTools
 * snapshots, browser-extension DOM scans, and BFCache restores cannot
 * recover what was never in client state.
 *
 * Constraints:
 *   - httpOnly + secure (prod) + sameSite=lax — cookie is server-read only.
 *   - path="/settings/tokens" — every other surface is blind to it.
 *   - max-age=120 — humans copy the secret in seconds; longer than
 *     that is a debugging convenience that becomes an exposure window.
 *   - Single-use: the reveal page deletes the cookie before returning.
 *
 * Threat model: an attacker who already has the user's session cookie
 * could observe the reveal cookie too — but that attacker would not
 * be running the mint flow, so the cookie wouldn't exist for them.
 * Cross-site script injection is excluded by httpOnly.
 */

import { createCipheriv, createDecipheriv, createHash, randomBytes } from "node:crypto";
import { cookies } from "next/headers";

export const REVEAL_COOKIE = "cdp_token_reveal";
const TTL_SECONDS = 120;
const ALGO = "aes-256-gcm";
const IV_LEN = 12;
const TAG_LEN = 16;
const COOKIE_PATH = "/settings/tokens";

function deriveKey(): Buffer {
  const secret = process.env.AUTH_SECRET;
  if (!secret) {
    throw new Error(
      "AUTH_SECRET is not set; cannot reveal a freshly-minted token.",
    );
  }
  return createHash("sha256")
    .update("token-reveal:" + secret)
    .digest();
}

function encrypt(plaintext: string): string {
  const key = deriveKey();
  const iv = randomBytes(IV_LEN);
  const cipher = createCipheriv(ALGO, key, iv);
  const ct = Buffer.concat([cipher.update(plaintext, "utf8"), cipher.final()]);
  const tag = cipher.getAuthTag();
  return [
    iv.toString("base64url"),
    ct.toString("base64url"),
    tag.toString("base64url"),
  ].join(".");
}

function decrypt(blob: string): string | null {
  const parts = blob.split(".");
  if (parts.length !== 3) return null;
  const [ivPart, ctPart, tagPart] = parts;
  try {
    const iv = Buffer.from(ivPart, "base64url");
    const ct = Buffer.from(ctPart, "base64url");
    const tag = Buffer.from(tagPart, "base64url");
    if (iv.length !== IV_LEN || tag.length !== TAG_LEN) return null;
    const key = deriveKey();
    const decipher = createDecipheriv(ALGO, key, iv);
    decipher.setAuthTag(tag);
    const pt = Buffer.concat([decipher.update(ct), decipher.final()]);
    return pt.toString("utf8");
  } catch {
    return null;
  }
}

type RevealPayload = {
  plaintext: string;
  tokenName: string;
  displayPrefix: string;
  /**
   * The user id the token was minted for. The reveal page checks this
   * against the current session and refuses to render if they differ —
   * defends against same-browser logout-then-login or an explicit
   * account switch happening within the 120s TTL.
   */
  userId: string;
};

export async function setRevealCookie(payload: RevealPayload): Promise<void> {
  const blob = encrypt(JSON.stringify(payload));
  const jar = await cookies();
  jar.set(REVEAL_COOKIE, blob, {
    httpOnly: true,
    secure: process.env.NODE_ENV === "production",
    sameSite: "lax",
    path: COOKIE_PATH,
    maxAge: TTL_SECONDS,
  });
}

/**
 * Read the reveal cookie WITHOUT modifying it. Server Components
 * may call this freely — Next.js 15 only forbids cookie *writes*
 * from Server Components, not reads. Returns the payload only when
 * the minting user matches `expectedUserId`; an anonymous reveal-
 * page visit with a signed-out session returns null even if a
 * cookie exists, defending against same-browser logout-then-visit.
 *
 * Single-use is enforced by the companion `deleteRevealCookie()`
 * which runs from a Server Action (the "Done" button) and by the
 * 120-second cookie TTL as a backstop. The previous shape combined
 * read+delete in `consumeRevealCookie`, which threw under Next.js
 * 15's "Cookies can only be modified in a Server Action or Route
 * Handler" enforcement.
 */
export async function peekRevealCookie(
  expectedUserId: string,
): Promise<RevealPayload | null> {
  const jar = await cookies();
  const blob = jar.get(REVEAL_COOKIE)?.value;
  if (!blob) return null;
  const json = decrypt(blob);
  if (!json) return null;
  try {
    const obj = JSON.parse(json) as RevealPayload;
    if (
      typeof obj.plaintext !== "string" ||
      typeof obj.tokenName !== "string" ||
      typeof obj.displayPrefix !== "string" ||
      typeof obj.userId !== "string"
    ) {
      return null;
    }
    if (obj.userId !== expectedUserId) {
      return null;
    }
    return obj;
  } catch {
    return null;
  }
}

/**
 * Delete the reveal cookie. MUST be called only from a Server
 * Action or Route Handler — Server Components throw under Next.js
 * 15 when modifying cookies. The reveal page wires this to the
 * "Done" button via a Server Action; the 120-second cookie TTL
 * is the backstop for users who navigate away without clicking.
 */
export async function deleteRevealCookie(): Promise<void> {
  const jar = await cookies();
  jar.delete({ name: REVEAL_COOKIE, path: COOKIE_PATH });
}
