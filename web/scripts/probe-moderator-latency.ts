/**
 * One-off probe: measure raw gpt-4o-mini latency with the policy
 * moderator's exact prompt + structured-output config, but WITHOUT
 * the AbortController timeout. Tells us whether the 1500ms ceiling
 * is realistic or whether we need to bump it.
 *
 *   pnpm tsx --env-file=.env.local scripts/probe-moderator-latency.ts
 */

import OpenAI from "openai";
import {
  POLICY_RESPONSE_JSON_SCHEMA,
  buildSystemPrompt,
  buildUserPrompt,
} from "@/lib/moderation/prompt";
import { POLICY_MODEL } from "@/lib/moderation/types";

const client = new OpenAI({ apiKey: process.env.OPENAI_API_KEY });

const cases = [
  {
    label: "short comment",
    content: { kind: "comment" as const, title: "", body: "Nice writeup." },
  },
  {
    label: "tutorial submission",
    content: {
      kind: "submission" as const,
      title: "Tutorial: building an eval harness for legal-doc summarization",
      body: "Step 1: gather 50 documents with hand-written summaries. Step 2: define your judge prompt. Step 3: run pairwise comparison. We hit 87% agreement.",
    },
  },
  {
    label: "spam submission",
    content: {
      kind: "submission" as const,
      title: "AMAZING DEAL — buy followers cheap",
      body: "Get 10K followers for $5! Visit www.spammy.example/promo. No questions asked.",
    },
  },
  {
    label: "doxxing reject",
    content: {
      kind: "comment" as const,
      title: "",
      body: "I know who you are. Your real name is Jane Doe, you live at 1234 Maple St, Apt 4B, Springfield, your phone is 555-0142 and your government ID number ends in 8472. Stop posting.",
    },
  },
  {
    label: "security research (FP risk)",
    content: {
      kind: "submission" as const,
      title: "Reverse-engineering the EvilCorp ransomware payload",
      body: "We disassembled the EvilCorp.bin sample from the September incident. The payload uses RSA-2048 with a hardcoded public key and AES-256-CBC for file encryption. Decryption requires the private key from the C2 server. Mitigation: snapshot Volume Shadow Copies before the kill switch fires (the malware shells out to vssadmin delete shadows /all).",
    },
  },
];

(async () => {
  for (const c of cases) {
    const t0 = Date.now();
    try {
      const completion = await client.chat.completions.create({
        model: POLICY_MODEL,
        messages: [
          { role: "system", content: buildSystemPrompt() },
          { role: "user", content: buildUserPrompt(c.content) },
        ],
        response_format: {
          type: "json_schema",
          json_schema: POLICY_RESPONSE_JSON_SCHEMA,
        },
        temperature: 0,
      });
      const elapsed = Date.now() - t0;
      const usage = completion.usage;
      const raw = completion.choices[0]?.message?.content ?? "";
      console.log(
        JSON.stringify(
          {
            label: c.label,
            elapsedMs: elapsed,
            promptTokens: usage?.prompt_tokens,
            completionTokens: usage?.completion_tokens,
            verdict: JSON.parse(raw),
          },
          null,
          2,
        ),
      );
    } catch (err) {
      const elapsed = Date.now() - t0;
      console.error(
        `${c.label}: failed after ${elapsed}ms — ${err instanceof Error ? err.message : String(err)}`,
      );
    }
  }
})();
