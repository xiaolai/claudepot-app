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
// The registry is read with Vite's `?raw` + JSON.parse rather than a
// glob import: these files live outside `src/`, and the eager-glob form
// would bake a build-time file list into the bundle. Adding a domain
// file must fail here, not silently pass.
import foundation from "../../crates/claudepot-core/testdata/error-codes/00-foundation.json?raw";
import account from "../../crates/claudepot-core/testdata/error-codes/10-account.json?raw";
import project from "../../crates/claudepot-core/testdata/error-codes/20-project.json?raw";
import config from "../../crates/claudepot-core/testdata/error-codes/30-config.json?raw";
import agent from "../../crates/claudepot-core/testdata/error-codes/40-agent.json?raw";
import sharedMemory from "../../crates/claudepot-core/testdata/error-codes/50-shared-memory.json?raw";
import miscCommands from "../../crates/claudepot-core/testdata/error-codes/70-misc-commands.json?raw";
import coreCommands from "../../crates/claudepot-core/testdata/error-codes/60-core-commands.json?raw";

interface CodeVector {
  code: string;
  owner: string;
  params: string[];
  message: string;
}

const REGISTRY: CodeVector[] = [
  foundation,
  account,
  project,
  config,
  agent,
  sharedMemory,
  coreCommands,
  miscCommands,
]
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
