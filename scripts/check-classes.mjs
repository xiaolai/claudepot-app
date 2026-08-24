// Is the renderer actually wired to the design system?
//
// Two checks, one question. A `className` with no rule behind it, and a
// form control with no chrome at all, fail the same way: valid markup,
// no error anywhere, and a surface that renders wrong.
//
// ## 1. Does every `className` have a rule behind it?
//
// A class name with no CSS rule is valid HTML, invisible to `tsc`, and
// invisible to a render test that asserts on text. So a whole pane can
// ship as unstyled markup with every gate green — which is exactly what
// happened: `RemotePane` was written against `pane`, `pane-block`,
// `pane-intro`, `pane-warning`, `pane-error`, `pane-actions`,
// `remote-devices` and `status-chip`, none of which existed.
//
// That was the second instance, not the first. `QuickPromptsPane` had
// been rendering a dead `pane` since it was written, and
// `ProtectedPathsPane` carries a comment from an earlier pass that
// found `className="btn outline"` doing nothing and fixed it by hand.
// Two of the same shape is one defect, so this is the mechanical check
// rather than a third careful reading.
//
// ## 2. Does every text field draw chrome?
//
// `tokens.css` gives `input, textarea` only `font` and `color` — no
// border reset, no background, no radius. So a bare `<input>` renders
// with the user-agent border, which in WebKit is a 2px INSET bevel, and
// a bare `<textarea>` gets a 1px grey rule. `QuickPromptsPane` had one
// of each, six inches from fields that went through `Input` and
// therefore had the design system's hairline.
//
// The panel hit this independently — `panel/src/controls.css` documents
// the same measurement — which is the second instance that makes it a
// class rather than a slip.
//
// A bare control passes only if it carries a className that check 1
// finds a rule for. `CommandPalette`'s `.palette-input` is the one such
// case and is deliberate: it is styled, just not by a primitive.
//
//   node scripts/check-classes.mjs [--self-test]
//
// ## What counts as "defined"
//
// Any `.name` appearing in a stylesheet under `src/styles/`, plus
// `src/App.css` and `index.html`. That is deliberately loose — the goal
// is to catch a name nothing anywhere styles, not to prove the selector
// would match. A tighter parse would reject the compound and
// pseudo-class forms this codebase legitimately uses.
//
// ## What is skipped, and why
//
// - **Comments.** `ProtectedPathsPane` quotes `className="btn outline"`
//   inside a comment explaining why it was removed. The first version of
//   this scan reported that as a live finding.
// - **Dynamic values.** `className={...}` with an expression is out of
//   reach; only string literals are read. A template literal with an
//   interpolation is skipped whole, since half a class name is worse
//   than none. The panel's own `statusbar-chip${x ? " warn" : ""}` shape
//   is therefore unchecked here — that file is not in `src/` anyway.
// - **`lucide*`.** `lucide-react` stamps `class="lucide lucide-<name>"`
//   onto every icon SVG. Those belong to the library, not to `styles/`.
// - **Test files.** They render fixtures, not product markup.

import { readFileSync, readdirSync, statSync, writeFileSync, mkdtempSync, rmSync } from "node:fs";
import { join, sep } from "node:path";
import { tmpdir } from "node:os";

const SELF_TEST = process.argv.includes("--self-test");

/** Every class the library owns rather than us. */
const VENDOR = /^lucide/;

function walk(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) walk(p, out);
    else out.push(p);
  }
  return out;
}

/** Strip block and line comments so quoted examples are not findings. */
function stripComments(src) {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, "")
    // Not preceded by `:` `"` `'` `` ` `` or `\` — so `https://` and a
    // string containing `//` survive.
    .replace(/(^|[^:"'`\\])\/\/[^\n]*/g, "$1");
}

/** `.name` occurrences across every stylesheet plus the HTML shell. */
function definedClasses(root) {
  const sheets = walk(join(root, "src/styles")).filter((f) => f.endsWith(".css"));
  let css = sheets.map((f) => readFileSync(f, "utf8")).join("\n");
  for (const extra of ["src/App.css", "index.html"]) {
    try {
      css += "\n" + readFileSync(join(root, extra), "utf8");
    } catch {
      // Optional: the self-test fixture has neither.
    }
  }
  return new Set([...css.matchAll(/\.([a-zA-Z][\w-]*)/g)].map((m) => m[1]));
}

/** Class names in string-literal `className` attributes. */
function usedClasses(root) {
  const files = walk(join(root, "src")).filter(
    (f) => /\.(tsx|ts)$/.test(f) && !/\.test\.(tsx|ts)$/.test(f),
  );
  const uses = new Map();
  for (const file of files) {
    const src = stripComments(readFileSync(file, "utf8"));
    // `className="a b"` and `className={`a b`}` with no interpolation.
    for (const m of src.matchAll(/className=(?:"([^"]*)"|\{`([^`$]*)`\})/g)) {
      for (const name of (m[1] ?? m[2]).trim().split(/\s+/)) {
        if (!name) continue;
        if (!uses.has(name)) uses.set(name, new Set());
        uses.get(name).add(file.slice(root.length + 1));
      }
    }
  }
  return uses;
}

/**
 * The attribute text of one JSX element, from after the tag name to the
 * `>` that closes it.
 *
 * Brace-aware, and that is the whole point: a naive scan to the first
 * `>` stops at the arrow in `onChange={(e) => …}`, which is present on
 * nearly every control — so it never reaches `style=` and reported 70
 * false positives on this repo. Strings are skipped too, so a `>` inside
 * a placeholder does not end the element either.
 */
function attributeText(src, from) {
  let depth = 0;
  let quote = null;
  for (let i = from; i < src.length; i += 1) {
    const c = src[i];
    if (quote) {
      if (c === quote && src[i - 1] !== "\\") quote = null;
      continue;
    }
    if (c === '"' || c === "'" || c === "`") quote = c;
    else if (c === "{") depth += 1;
    else if (c === "}") depth -= 1;
    else if (c === ">" && depth === 0) return src.slice(from, i);
  }
  return src.slice(from);
}

/**
 * Bare `<input>` / `<textarea>` with no styled className.
 *
 * Primitives are excluded: `Input` and `Textarea` ARE the bare elements,
 * and they clear the UA border inline via `fieldChrome`.
 */
function bareControls(root, defined) {
  const files = walk(join(root, "src")).filter(
    (f) =>
      /\.tsx$/.test(f) &&
      !/\.test\.tsx$/.test(f) &&
      !f.includes(`primitives${sep}`),
  );
  const findings = [];
  for (const file of files) {
    const src = stripComments(readFileSync(file, "utf8"));
    for (const m of src.matchAll(/<(input|textarea)[\s/>]/g)) {
      const attrs = attributeText(src, m.index + m[0].length - 1);
      const cls = /className="([^"]*)"/.exec(attrs);
      const styled = cls?.[1]
        .trim()
        .split(/\s+/)
        .some((c) => c && defined.has(c));
      // An inline `style=` is acceptable evidence the author drew the
      // chrome deliberately — `AgentForm`'s `style={inputStyle(...)}` is
      // the common shape.
      // `checkbox` / `radio` / `file` want the native control: their
      // chrome IS the UA's, and the app relies on it — `UpdatesPanel`
      // carries a note about native checkbox rendering in the Tauri
      // webview. Only text-entry controls are in scope.
      const nativeByDesign = /type="(checkbox|radio|file)"/.test(attrs);
      if (!nativeByDesign && !styled && !attrs.includes("style=")) {
        findings.push({ tag: m[1], file: file.slice(root.length + 1) });
      }
    }
  }
  return findings;
}

function check(root) {
  const defined = definedClasses(root);
  const used = usedClasses(root);
  const orphans = [...used]
    .filter(([name]) => !VENDOR.test(name))
    .filter(([name]) => !defined.has(name));
  return { defined, used, orphans, bare: bareControls(root, defined) };
}

if (SELF_TEST) {
  // A fixture with one styled class and one invented one, through the
  // same code path. A gate nobody has watched go red is indistinguishable
  // from one that cannot.
  const dir = mkdtempSync(join(tmpdir(), "check-classes-selftest-"));
  try {
    const styles = join(dir, "src/styles");
    readdirSync(dir); // ensure dir exists before nested mkdir
    for (const d of ["src", "src/styles"]) {
      try {
        statSync(join(dir, d));
      } catch {
        const { mkdirSync } = await import("node:fs");
        mkdirSync(join(dir, d), { recursive: true });
      }
    }
    writeFileSync(join(styles, "x.css"), ".real { color: red }\n");
    writeFileSync(
      join(dir, "src/Thing.tsx"),
      'export const A = () => <div className="real invented" />;\n' +
        // Three controls: one bare (a finding), one with a styled class
        // and one with an inline style (both fine).
        'export const B = () => <input />;\n' +
        'export const C = () => <input className="real" />;\n' +
        'export const D = () => <textarea style={{ border: "none" }} />;\n',
    );
    const { orphans, bare } = check(dir);
    if (bare.length !== 1 || bare[0].tag !== "input") {
      console.error(
        `self-test FAILED: expected exactly one bare control, got ${JSON.stringify(bare)}`,
      );
      process.exit(1);
    }
    const names = orphans.map(([n]) => n);
    if (!names.includes("invented")) {
      console.error(`self-test FAILED: an invented class was not reported (got ${names})`);
      process.exit(1);
    }
    if (names.includes("real")) {
      console.error("self-test FAILED: a defined class was reported as an orphan");
      process.exit(1);
    }
    console.log(
      "self-test ok — fires on an invented class and a bare control, " +
        "spares a defined class, a styled control and an inline-styled one",
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  process.exit(0);
}

const { defined, used, orphans, bare } = check(process.cwd());

// Green is not evidence that anything happened: an empty corpus on
// either side would report zero orphans.
if (defined.size < 100 || used.size < 100) {
  console.error(
    `classes: refusing a vacuous pass — found ${defined.size} defined and ${used.size} used`,
  );
  process.exit(1);
}

if (orphans.length) {
  console.error(`classes: ${orphans.length} class(es) with no rule in any stylesheet\n`);
  for (const [name, files] of orphans.sort()) {
    console.error(`  .${name}`);
    for (const f of files) console.error(`      ${f}`);
  }
  console.error(
    "\nEither add the rule to a shard under src/styles/components/, or drop the\n" +
      "class — a className with no rule renders as unstyled markup and nothing\n" +
      "else will tell you.",
  );
  process.exit(1);
}

if (bare.length) {
  console.error(`classes: ${bare.length} form control(s) with no chrome\n`);
  for (const { tag, file } of bare) console.error(`  <${tag}>  ${file}`);
  console.error(
    "\n`tokens.css` gives `input, textarea` only `font` and `color`, so a bare\n" +
      "one renders with the user-agent border. Use the `Input` / `Textarea`\n" +
      "primitives, or give it a class with a rule.",
  );
  process.exit(1);
}

console.log(
  `classes OK — ${used.size} class names used, all defined among ${defined.size} ` +
    `in src/styles; no bare form controls`,
);
