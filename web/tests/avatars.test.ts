/**
 * Tests for src/lib/avatar-validation.ts.
 *
 *   pnpm tsx tests/avatars.test.ts
 *
 * Imports only the validation primitives (no DB / blob dependencies)
 * so the unit suite runs without DATABASE_URL. The setAvatar /
 * clearAvatar persistence functions live in lib/avatars.ts and
 * exercise Vercel Blob + the DB; integration coverage for those
 * needs an integration harness, out of scope here.
 *
 * The load-bearing security check is the magic-byte detection — if
 * the client claims image/png but sends a non-PNG, the upload must
 * fail at the lib layer before bytes ever land in blob storage.
 * These tests pin that contract.
 */

import assert from "node:assert/strict";
import {
  ALLOWED_AVATAR_TYPES,
  detectAvatarMagicType,
  MAX_AVATAR_BYTES,
} from "../src/lib/avatar-validation";

let passed = 0;
let failed = 0;
function test(name: string, fn: () => void) {
  try {
    fn();
    console.log(`PASS  ${name}`);
    passed += 1;
  } catch (err) {
    console.error(`FAIL  ${name}`);
    console.error(`      ${err instanceof Error ? err.message : String(err)}`);
    failed += 1;
  }
}

/* ── Constants ───────────────────────────────────────────────── */

test("MAX_AVATAR_BYTES is 2 MB", () => {
  assert.equal(MAX_AVATAR_BYTES, 2 * 1024 * 1024);
});

test("ALLOWED_AVATAR_TYPES is PNG, JPEG, WebP — and SVG is NOT included", () => {
  assert.deepEqual(
    [...ALLOWED_AVATAR_TYPES].sort(),
    ["image/jpeg", "image/png", "image/webp"],
  );
  assert.equal(
    ALLOWED_AVATAR_TYPES.includes(
      "image/svg+xml" as never,
    ),
    false,
  );
});

/* ── Magic-byte detection — happy path ──────────────────────── */

const PNG_PREFIX = new Uint8Array([
  0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
  0x00, 0x00, 0x00, 0x0d, // 4-byte filler so length >= 12
]);
const JPEG_PREFIX = new Uint8Array([
  0xff, 0xd8, 0xff, 0xe0,
  0x00, 0x10, 0x4a, 0x46, 0x49, 0x46, 0x00, 0x01,
]);
const WEBP_PREFIX = new Uint8Array([
  0x52, 0x49, 0x46, 0x46, // "RIFF"
  0x00, 0x00, 0x00, 0x00, // size placeholder
  0x57, 0x45, 0x42, 0x50, // "WEBP"
]);

test("detects PNG by magic bytes", () => {
  assert.equal(detectAvatarMagicType(PNG_PREFIX), "image/png");
});

test("detects JPEG by magic bytes", () => {
  assert.equal(detectAvatarMagicType(JPEG_PREFIX), "image/jpeg");
});

test("detects WebP by magic bytes (RIFF + WEBP)", () => {
  assert.equal(detectAvatarMagicType(WEBP_PREFIX), "image/webp");
});

/* ── Magic-byte detection — reject paths ────────────────────── */

test("returns null for too-short input (need >= 12 bytes for WebP signature)", () => {
  const short = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a]);
  assert.equal(detectAvatarMagicType(short), null);
});

test("returns null for SVG (intentionally not supported)", () => {
  // SVG starts with `<` (0x3c) or `<?xml` if the prolog is present.
  const svg = new Uint8Array([
    0x3c, 0x73, 0x76, 0x67, 0x20, 0x78, 0x6d, 0x6c,
    0x6e, 0x73, 0x3d, 0x22, 0x68,
  ]);
  assert.equal(detectAvatarMagicType(svg), null);
});

test("returns null for arbitrary garbage", () => {
  const garbage = new Uint8Array([
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
    0x08, 0x09, 0x0a, 0x0b,
  ]);
  assert.equal(detectAvatarMagicType(garbage), null);
});

test("returns null for a polyglot that LOOKS like RIFF but isn't WebP", () => {
  // RIFF...AVI (audio/video, not WebP) — same RIFF prefix but
  // different format marker at byte 8.
  const avi = new Uint8Array([
    0x52, 0x49, 0x46, 0x46,
    0x00, 0x00, 0x00, 0x00,
    0x41, 0x56, 0x49, 0x20, // "AVI " not "WEBP"
  ]);
  assert.equal(detectAvatarMagicType(avi), null);
});

test("returns null for empty input", () => {
  assert.equal(detectAvatarMagicType(new Uint8Array(0)), null);
});

/* ── The load-bearing contract ───────────────────────────────── */

test("client claims PNG but sends JPEG bytes — detected as JPEG, mismatch", () => {
  // setAvatar requires both:
  //   1. file.type ∈ ALLOWED_AVATAR_TYPES
  //   2. detectAvatarMagicType(bytes) === file.type
  // Pinning case (2): a content-type-claim of image/png with JPEG
  // bytes detects as image/jpeg, which fails the equality check.
  const detected = detectAvatarMagicType(JPEG_PREFIX);
  assert.equal(detected, "image/jpeg");
  const declared = "image/png";
  assert.notEqual(detected, declared); // mismatch → upload rejected
});

test("client claims WebP but sends PNG bytes — detected as PNG, mismatch", () => {
  const detected = detectAvatarMagicType(PNG_PREFIX);
  assert.equal(detected, "image/png");
  const declared = "image/webp";
  assert.notEqual(detected, declared); // mismatch → upload rejected
});

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
