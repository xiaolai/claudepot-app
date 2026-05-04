/**
 * Round-trip tests for the digest unsubscribe HMAC token.
 *
 * Run via `pnpm test`.
 */

process.env.AUTH_SECRET = "test-secret-only-for-this-suite";

import {
  signUnsubscribeToken,
  verifyUnsubscribeToken,
  buildUnsubscribeUrl,
} from "../src/lib/email/unsubscribe";

let passed = 0;
let failed = 0;

function check(name: string, ok: boolean, detail?: string) {
  if (ok) {
    console.log(`PASS  ${name}`);
    passed++;
  } else {
    console.error(`FAIL  ${name}${detail ? `: ${detail}` : ""}`);
    failed++;
  }
}

const userA = "11111111-1111-4111-8111-111111111111";
const userB = "22222222-2222-4222-8222-222222222222";

const tokenA = signUnsubscribeToken(userA);
const tokenB = signUnsubscribeToken(userB);

check("token signs as non-empty string", typeof tokenA === "string" && (tokenA?.length ?? 0) > 10);
check("verify accepts the canonical token", verifyUnsubscribeToken(userA, tokenA!));
check("verify rejects a different user's token", !verifyUnsubscribeToken(userA, tokenB!));
check(
  "verify rejects a tampered token",
  !verifyUnsubscribeToken(userA, tokenA!.slice(0, -1) + "_"),
);
check("verify rejects an empty token", !verifyUnsubscribeToken(userA, ""));
check("verify rejects a non-string", !verifyUnsubscribeToken(userA, undefined as unknown as string));

const url = buildUnsubscribeUrl("https://example.com", userA);
check("buildUnsubscribeUrl returns absolute https URL", !!url && url.startsWith("https://example.com/api/unsubscribe/digest?"));
check("URL carries the userId", url?.includes(`u=${userA}`) ?? false);
check("URL carries a t param", url?.includes("&t=") ?? false);

// Without AUTH_SECRET: signing must return null rather than crashing.
const oldSecret = process.env.AUTH_SECRET;
delete process.env.AUTH_SECRET;
check("signing returns null without AUTH_SECRET", signUnsubscribeToken(userA) === null);
check(
  "verify returns false without AUTH_SECRET",
  !verifyUnsubscribeToken(userA, tokenA!),
);
process.env.AUTH_SECRET = oldSecret;

console.log(`\n${passed} passed, ${failed} failed`);
if (failed > 0) process.exit(1);
