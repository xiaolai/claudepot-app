#!/usr/bin/env tsx
/**
 * redact.ts — strip secrets from env files / log output before
 * pasting into chat, bug reports, or screenshots. Bridges
 * `.env.local` (gitignored, secret-bearing) into a shareable form
 * without leaking credentials.
 *
 * Usage:
 *   pnpm exec tsx scripts/redact.ts < .env.local
 *   pnpm exec tsx scripts/redact.ts .env.local
 *   <command-that-might-leak> 2>&1 | pnpm exec tsx scripts/redact.ts
 *   pnpm exec tsx scripts/redact.ts --self-test
 *
 * Rules (applied in order; later rules see earlier replacements):
 *   1. Connection-string userinfo:
 *        scheme://user:pass@host  →  scheme://[redacted]:[redacted]@host
 *   2. URL userinfo without password:
 *        scheme://token@host       →  scheme://[redacted]@host
 *   3. KEY=VALUE for sensitive keys (DATABASE_URL, *_TOKEN, *_KEY,
 *      *_SECRET, *_PASSWORD, *_CREDENTIALS, AUTH_*).
 *   4. Token shapes: sk-ant-…, sk-…, npg_…, ghp_…/gho_…/ghu_…/ghs_…/ghr_…,
 *      AKIA… (AWS), xox[bapors]-… (Slack), JWTs (eyJ.eyJ.…),
 *      `Bearer <token>`.
 *
 * The helper is purely a stream filter — never writes to disk,
 * never makes network calls. Safe to commit and run from any path.
 */

import { readFileSync } from "node:fs";

type Rule = [RegExp, string | ((match: string, ...groups: string[]) => string)];

const RULES: Rule[] = [
  // 1. URL userinfo with password.
  [
    /([a-z][a-z0-9+\-.]*:\/\/)([^:/@\s]+):([^@\s]+)@/gi,
    "$1[redacted]:[redacted]@",
  ],
  // 2. URL userinfo without password (Bearer-in-URL pattern).
  [/([a-z][a-z0-9+\-.]*:\/\/)([^@/\s:]+)@/gi, "$1[redacted]@"],
  // 3. Sensitive env-style KEY=VALUE lines. Match the right-hand side
  //    only — keep the key visible so the redacted output stays useful.
  //    Skips NEXT_PUBLIC_* (Next.js convention: explicitly browser-
  //    exposed). Bare `_URL` is intentionally NOT in the list; URL
  //    values with userinfo are caught by rule 1, and most `_URL`
  //    keys are public (SITE_URL, API_URL, CALLBACK_URL, etc.).
  [
    /^(?!NEXT_PUBLIC_)([A-Z][A-Z0-9_]*(?:DATABASE_URL|REDIS_URL|MONGO_URL|MONGODB_URL|POSTGRES_URL|POSTGRESQL_URL|AMQP_URL|RABBITMQ_URL|KAFKA_URL|_TOKEN|_KEY|_SECRET|_PASSWORD|_PASS|_PWD|_CREDENTIALS|_DSN|AUTH_SECRET|API_KEY|WEBHOOK_SECRET))\s*=\s*"?([^"\n]+)"?$/gim,
    (_match, key) => `${key}=[redacted]`,
  ],
  // 4a. Anthropic API keys.
  [/sk-ant-[A-Za-z0-9_-]+/g, "sk-ant-[redacted]"],
  // 4b. Generic OpenAI-style keys (sk- + ≥32 chars; runs after #4a so
  //     ant- variants are already masked).
  [/\bsk-[A-Za-z0-9]{20,}/g, "sk-[redacted]"],
  // 4c. Neon role passwords.
  [/\bnpg_[A-Za-z0-9]+/g, "npg_[redacted]"],
  // 4d. GitHub PAT / OAuth / refresh / server-to-server tokens.
  [/\bgh[pousr]_[A-Za-z0-9]{20,}/g, "gh_[redacted]"],
  // 4e. AWS access keys.
  [/\bAKIA[0-9A-Z]{16}\b/g, "AKIA[redacted]"],
  // 4f. Slack tokens.
  [/\bxox[baprs]-[A-Za-z0-9-]{10,}/g, "xox-[redacted]"],
  // 4g. JWTs (3 base64url segments).
  [
    /\beyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_.+/=-]+/g,
    "eyJ[redacted-jwt]",
  ],
  // 4h. Bearer header values.
  [/(Bearer\s+)([A-Za-z0-9._\-=+/]{8,})/g, "$1[redacted]"],
];

export function redact(input: string): string {
  let out = input;
  for (const [re, repl] of RULES) {
    out = typeof repl === "function" ? out.replace(re, repl) : out.replace(re, repl);
  }
  return out;
}

/* ── Self-test ──────────────────────────────────────────────── */

const SELF_TEST_CASES: Array<[string, string]> = [
  // [input, must-not-contain]
  [
    'NEON_DATABASE_URL="postgresql://neondb_owner:npg_3FGAx8zbSMsr@ep-foo.aws.neon.tech/neondb?sslmode=require"',
    "npg_3FGAx8zbSMsr",
  ],
  [
    "Connection string: postgresql://admin:hunter2@db.example.com:5432/app",
    "hunter2",
  ],
  ["Authorization: Bearer abc123def456ghi789jklmnop", "abc123def456ghi789jklmnop"],
  ["ANTHROPIC_API_KEY=sk-ant-oat01-AbcDefGhi-jkl_mno", "sk-ant-oat01-AbcDefGhi-jkl_mno"],
  ["GITHUB_TOKEN=ghp_abcdefghij1234567890ABCDEFGHIJ0987654321", "ghp_abcdefghij"],
  ["AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE", "AKIAIOSFODNN7EXAMPLE"],
  ["SLACK_BOT_TOKEN=xoxb-1234-abcd-EFGH5678ijkl", "xoxb-1234"],
  [
    "Header: eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
    "SflKxwRJ",
  ],
];

// Inputs that MUST pass through unchanged.
const PASS_THROUGH_CASES: string[] = [
  'NEXT_PUBLIC_SITE_URL="https://claudepot.com"',
  "NEXT_PUBLIC_API_KEY=public-recaptcha-site-key-12345",
  "All systems operational. Free quota: 10,000 ops/month.",
  "Visit https://docs.anthropic.com for the latest API reference.",
  "GITHUB_OAUTH_CLIENT_ID=Ov23lidOCc8wcAOw5ujO",
];

function selfTest(): number {
  let fails = 0;
  for (const [input, mustNotContain] of SELF_TEST_CASES) {
    const out = redact(input);
    if (out.includes(mustNotContain)) {
      fails += 1;
      console.error(`FAIL: input still contains ${mustNotContain.slice(0, 12)}…`);
      console.error(`      input:  ${input}`);
      console.error(`      output: ${out}`);
    }
  }
  // Pass-through cases must round-trip unchanged.
  for (const benign of PASS_THROUGH_CASES) {
    const out = redact(benign);
    if (out !== benign) {
      fails += 1;
      console.error("FAIL: benign text was modified");
      console.error(`      input:  ${benign}`);
      console.error(`      output: ${out}`);
    }
  }
  if (fails === 0) {
    console.log(
      `OK — ${SELF_TEST_CASES.length} redaction + ` +
        `${PASS_THROUGH_CASES.length} pass-through cases passed.`,
    );
  }
  return fails;
}

/* ── CLI entry ──────────────────────────────────────────────── */

async function main() {
  const args = process.argv.slice(2);

  if (args.includes("--self-test")) {
    process.exit(selfTest() === 0 ? 0 : 1);
  }

  let input: string;
  if (args.length > 0 && !args[0].startsWith("-")) {
    input = readFileSync(args[0], "utf8");
  } else {
    const chunks: Buffer[] = [];
    for await (const c of process.stdin) chunks.push(c as Buffer);
    input = Buffer.concat(chunks).toString("utf8");
  }

  process.stdout.write(redact(input));
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
