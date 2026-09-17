import { describe, expect, it } from "vitest";
import { sections } from "./registry";

/**
 * `lib/shortcutBindings.ts` documents ⌘R as "Refresh this section". For
 * most of the app's life only Accounts and Projects bound it — eight
 * sections showed refresh buttons and ignored the key, which then fell
 * through to the webview. Each registry entry now says where ⌘R is bound,
 * and this checks every file it names actually binds it.
 */

/**
 * Drop `//` and block comments so a mention in prose is not a binding.
 * A left-to-right scan rather than two regexes, on purpose: the first
 * version stripped block comments first, so the `/*` inside a LINE comment
 * in ConfigSection (`~/.claude/skills/*`) opened a block that ran to the
 * next `*` + `/` and swallowed the real binding. Strings are skipped for
 * the same reason.
 */
function stripComments(src: string): string {
  let out = "";
  let quote: string | null = null;
  for (let i = 0; i < src.length; i++) {
    const c = src[i];
    const next = src[i + 1];
    if (quote) {
      out += c;
      if (c === "\\") {
        out += next ?? "";
        i++;
      } else if (c === quote) {
        quote = null;
      }
      continue;
    }
    if (c === '"' || c === "'" || c === "`") {
      quote = c;
      out += c;
    } else if (c === "/" && next === "/") {
      while (i < src.length && src[i] !== "\n") i++;
      out += "\n";
    } else if (c === "/" && next === "*") {
      const end = src.indexOf("*/", i + 2);
      i = end < 0 ? src.length : end + 1;
    } else {
      out += c;
    }
  }
  return out;
}

/** True when some `useGlobalShortcuts(...)` call passes `onRefresh`. */
export function bindsRefresh(src: string): boolean {
  const code = stripComments(src);
  const needle = "useGlobalShortcuts(";
  let from = 0;
  for (;;) {
    const at = code.indexOf(needle, from);
    if (at < 0) return false;
    let depth = 0;
    let end = at + needle.length - 1;
    for (; end < code.length; end++) {
      if (code[end] === "(") depth++;
      else if (code[end] === ")" && --depth === 0) break;
    }
    const args = code.slice(at + needle.length, end);
    // The declaration `export function useGlobalShortcuts(handlers: {`
    // is not a call site.
    const isDeclaration = /function\s+$/.test(code.slice(Math.max(0, at - 20), at));
    if (!isDeclaration && /\bonRefresh\b/.test(args)) return true;
    from = end + 1;
  }
}

// `?raw` glob, not node:fs — this tsconfig targets the browser and has
// no Node types. Same mechanism as `errorCodes.test.ts`.
const SOURCES = import.meta.glob("./**/*.{ts,tsx}", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

/** Source text for a repo-relative `src/sections/...` path. */
function read(rel: string): string {
  const prefix = "src/sections/";
  expect(rel.startsWith(prefix), `${rel} is outside src/sections`).toBe(true);
  const src = SOURCES[`./${rel.slice(prefix.length)}`];
  // Loud on a renamed file rather than testing `undefined`, and on an
  // empty read — a stubbed `?raw` would otherwise fail every file for a
  // reason that has nothing to do with ⌘R.
  expect(src, `${rel} is not a file`).toBeTypeOf("string");
  expect(src.length, `${rel} read as empty`).toBeGreaterThan(0);
  return src;
}

describe("⌘R has a declared home in every section", () => {
  it("every section says what ⌘R does", () => {
    for (const s of sections) {
      const r = s.refresh;
      const ok =
        ("boundIn" in r && r.boundIn.length > 0) ||
        ("none" in r && r.none.trim().length > 0);
      expect(ok, s.id).toBe(true);
    }
  });

  const bound = sections.flatMap((s) =>
    "boundIn" in s.refresh ? s.refresh.boundIn.map((f) => [s.id, f] as const) : [],
  );

  it("names enough files that a broken parse cannot pass vacuously", () => {
    expect(bound.length).toBeGreaterThanOrEqual(10);
  });

  it.each(bound)("%s: %s binds ⌘R", (_id, file) => {
    expect(bindsRefresh(read(file))).toBe(true);
  });
});

describe("bindsRefresh", () => {
  it("accepts a call that passes onRefresh, however it is spelled", () => {
    expect(bindsRefresh("useGlobalShortcuts({ onRefresh: () => void r() });")).toBe(true);
    expect(bindsRefresh("useGlobalShortcuts({\n  onRefresh: go,\n});")).toBe(true);
  });

  it("is not fooled by a block-comment opener inside a line comment or a string", () => {
    // The shape that broke the first version, verbatim in miniature.
    const inComment =
      "// every `~/.claude/skills/*` as\nuseGlobalShortcuts({ onRefresh: r });\n/** doc */";
    expect(bindsRefresh(inComment)).toBe(true);
    const inString = 'const glob = "src/**/*.md";\nuseGlobalShortcuts({ onRefresh: r });\n/** doc */';
    expect(bindsRefresh(inString)).toBe(true);
  });

  it("rejects a call without onRefresh, a comment, and the declaration", () => {
    expect(bindsRefresh("useGlobalShortcuts({ onAdd: add });")).toBe(false);
    expect(bindsRefresh("// useGlobalShortcuts({ onRefresh })")).toBe(false);
    expect(bindsRefresh("/* useGlobalShortcuts({ onRefresh }) */")).toBe(false);
    expect(
      bindsRefresh("export function useGlobalShortcuts(handlers: { onRefresh?: () => void }) {}"),
    ).toBe(false);
  });
});
