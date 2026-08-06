import { describe, expect, it } from "vitest";
import enErrors from "../locales/en/errors.json";
import zhErrors from "../locales/zh-CN/errors.json";

// The TypeScript half of the error-code cross-language lock.
//
// `crates/claudepot-core/testdata/error-codes/*.json` is the registry:
// Rust asserts every enum variant and every command-site code appears
// there (`claudepot_core::error_code` and `dto_error` own that half).
// This file closes the loop from the other side — every registered code
// must have a translatable sentence in BOTH locales.
//
// Why it has to be a test rather than a convention: a code with no
// catalog entry does not throw. `renderError` falls back to
// `ErrorDto.message`, the English text from Rust, so the failure mode
// is a Chinese UI that silently emits English error sentences — the
// exact thing the phase exists to remove, and invisible in an
// English-language test suite.
//
// The registry is globbed from the shard directory, not listed. It was a
// fixed list of `?raw` imports, on the reasoning that an eager glob would
// bake a build-time file list into the bundle — but this is a test file
// and is never bundled, and the list made the comment above a lie: a new
// `80-*.json` shard was silently unchecked, which is the exact failure
// this suite exists to prevent. The Rust side already treats
// `error-codes/*.json` as the registry; this now matches it.
const SHARDS = import.meta.glob(
  "../../crates/claudepot-core/testdata/error-codes/*.json",
  { eager: true, query: "?raw", import: "default" },
) as Record<string, string>;

interface CodeVector {
  code: string;
  owner: string;
  params: string[];
  message: string;
}

const REGISTRY: CodeVector[] = Object.values(SHARDS)
  .map((raw) => JSON.parse(raw) as { codes: CodeVector[] })
  .flatMap((f) => f.codes);

/** `keys.not_found` → the nested catalog value, or undefined. */
function lookup(cat: unknown, code: string): string | undefined {
  const [mod, variant] = code.split(".");
  const m = (cat as Record<string, unknown>)[mod];
  if (!m || typeof m !== "object") return undefined;
  const v = (m as Record<string, unknown>)[variant];
  return typeof v === "string" ? v : undefined;
}

/** `{{name}}` placeholders a sentence interpolates. */
function placeholders(s: string): Set<string> {
  return new Set([...s.matchAll(/\{\{\s*([\w.-]+)\s*[,}]/g)].map((m) => m[1]));
}

describe("error-code catalog lock", () => {
  it("the glob actually found the shard files", () => {
    // Without this, a wrong glob pattern yields an empty REGISTRY and
    // every "each code has a sentence" assertion below passes over zero
    // codes. Green would mean nothing happened.
    const names = Object.keys(SHARDS)
      .map((p) => p.split("/").pop())
      .sort();
    expect(names.length).toBeGreaterThanOrEqual(8);
    expect(names).toContain("00-foundation.json");
  });

  it("the registry is non-empty and free of duplicates", () => {
    expect(REGISTRY.length).toBeGreaterThan(400);
    const seen = new Set<string>();
    for (const v of REGISTRY) {
      expect(seen.has(v.code), `duplicate code ${v.code}`).toBe(false);
      seen.add(v.code);
    }
  });

  it("every registered code has an English sentence", () => {
    const missing = REGISTRY.filter((v) => !lookup(enErrors, v.code));
    expect(missing.map((v) => v.code)).toEqual([]);
  });

  it("every registered code has a zh-CN sentence", () => {
    // A missing entry here renders the Rust English fallback — silent,
    // and invisible to a suite running in English.
    const missing = REGISTRY.filter((v) => !lookup(zhErrors, v.code));
    expect(missing.map((v) => v.code)).toEqual([]);
  });

  it("no catalog entry exists for a code nothing raises", () => {
    // Dead copy rots: it is never rendered, so a wrong translation in
    // it is never noticed.
    const known = new Set(REGISTRY.map((v) => v.code));
    const orphans: string[] = [];
    for (const [mod, vars] of Object.entries(enErrors)) {
      for (const variant of Object.keys(vars as object)) {
        const code = `${mod}.${variant}`;
        if (!known.has(code)) orphans.push(code);
      }
    }
    expect(orphans).toEqual([]);
  });

  it("a sentence only interpolates params the error actually carries", () => {
    // `params()` is what Rust puts on the wire. A sentence referencing
    // {{path}} on an error that carries only {{detail}} renders the
    // literal braces to the user.
    const bad: string[] = [];
    for (const v of REGISTRY) {
      const carried = new Set(v.params);
      // `cause` is synthesized by the renderer, not by Rust's `params()`:
      // a wrapper that carries `cause_code` has its cause resolved
      // against this same catalog and interpolated as `{{cause}}`. See
      // `causeClause` in `src/lib/i18n-error.ts`.
      if (carried.has("cause_code")) carried.add("cause");
      for (const cat of [enErrors, zhErrors]) {
        const sentence = lookup(cat, v.code);
        if (!sentence) continue;
        for (const p of placeholders(sentence)) {
          if (!carried.has(p)) bad.push(`${v.code} uses {{${p}}}`);
        }
      }
    }
    expect(bad).toEqual([]);
  });

  it("a sentence using {{cause}} carries a cause to resolve", () => {
    // The converse of the rule above, and the more dangerous direction:
    // `{{cause}}` with no `cause_code` in params renders the literal
    // braces to the user, because nothing ever supplies that value.
    const bad: string[] = [];
    for (const v of REGISTRY) {
      for (const cat of [enErrors, zhErrors]) {
        const sentence = lookup(cat, v.code);
        if (!sentence || !placeholders(sentence).has("cause")) continue;
        if (!v.params.includes("cause_code")) {
          bad.push(`${v.code} interpolates {{cause}} but carries no cause_code`);
        }
      }
    }
    expect(bad).toEqual([]);
  });

  it("en and zh-CN agree on every sentence's placeholders", () => {
    const drift: string[] = [];
    for (const v of REGISTRY) {
      const en = lookup(enErrors, v.code);
      const zh = lookup(zhErrors, v.code);
      if (!en || !zh) continue;
      const a = [...placeholders(en)].sort().join(",");
      const b = [...placeholders(zh)].sort().join(",");
      if (a !== b) drift.push(`${v.code}: en(${a}) vs zh(${b})`);
    }
    expect(drift).toEqual([]);
  });

  it("no sentence leaks a token-shaped value", () => {
    // params never carry secrets (enforced Rust-side); this guards the
    // copy itself against an example token pasted into a sentence.
    for (const cat of [enErrors, zhErrors]) {
      for (const vars of Object.values(cat)) {
        for (const s of Object.values(vars as Record<string, string>)) {
          expect(s).not.toMatch(/sk-ant-[A-Za-z0-9_-]{4,}/);
        }
      }
    }
  });
});
