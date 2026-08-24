// Does every switch have an accessible name?
//
// A `<button role="switch">` whose content is a decorative `<span>` has
// no accessible name at all. It announces as "switch, not checked" with
// nothing saying what it switches — and the visible text sitting beside
// it is not a label, however obvious it looks on screen. Only
// `aria-label`, `aria-labelledby`, or the button's own text content is.
//
// Two shipped that way: `SettingsSection`'s `Toggle`, behind fourteen
// call sites, and `UpdatesPanel`'s, whose docstring asserted the label
// was "rendered as a sibling by the caller … same a11y semantics".
// `SettingToggleRow` — the canonical version of the same row — had it
// right the whole time, which is what makes this mechanical rather than
// a matter of taste: the correct pattern was already in the tree.
//
//   node scripts/check-a11y-names.mjs [--self-test]
//
// ## The rule is absolute: an aria attribute, not inferred content
//
// A `<button role="switch">Foo</button>` would be named by its text, so
// in principle the check could accept that. It does not, for two
// reasons. Every switch in this codebase is a pill with one decorative
// `aria-hidden` span inside, so the content path would be dead code
// covering nothing — and the version of this that tried to detect
// content let the real regression through: the `<span>`'s inline style
// object satisfied a "does it contain an expression" test, so removing
// `aria-label` from `SettingsSection`'s Toggle still reported OK.
// Watched, on the actual file, not a fixture. A gate that cannot catch
// the bug it was written for is worse than no gate, and the narrow rule
// has no such hole.
//
// ## Why only `role="switch"`
//
// It is the one control in this codebase that is routinely built from a
// `<button>` with no text. A native `<input type="checkbox">` gets its
// name from a wrapping or associated `<label>`, and the failure there is
// different in kind — the name is too LONG, because a wrapping label
// swallows the description too. That one is a judgement call about what
// counts as description, and judgement calls make bad gates; it is
// documented on `SettingToggleRow` instead.
//
// ## What is skipped
//
// Comments (a docstring quoting `role="switch"` is not a control) and
// test files (they render fixtures). Attribute scanning is brace- and
// string-aware, because a naive scan to the first `>` stops at the arrow
// in `onClick={() => …}` — a mistake this repo has already made once, in
// `check-classes.mjs`, where it produced 70 false positives.

import { readFileSync, readdirSync, statSync, writeFileSync, mkdtempSync, rmSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const SELF_TEST = process.argv.includes("--self-test");

function walk(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) walk(p, out);
    else out.push(p);
  }
  return out;
}

function stripComments(src) {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/(^|[^:"'`\\])\/\/[^\n]*/g, "$1");
}

/** The opening tag containing `at`, from its `<` to the `>` that closes it. */
function openingTag(src, at) {
  const start = src.lastIndexOf("<", at);
  let depth = 0;
  let quote = null;
  for (let i = start; i < src.length; i += 1) {
    const c = src[i];
    if (quote) {
      if (c === quote && src[i - 1] !== "\\") quote = null;
      continue;
    }
    if (c === '"' || c === "'" || c === "`") quote = c;
    else if (c === "{") depth += 1;
    else if (c === "}") depth -= 1;
    else if (c === ">" && depth === 0) return src.slice(start, i + 1);
  }
  return src.slice(start);
}

function check(root) {
  const files = walk(join(root, "src")).filter(
    (f) => /\.tsx$/.test(f) && !/\.test\.tsx$/.test(f),
  );
  const findings = [];
  let switches = 0;
  for (const file of files) {
    const src = stripComments(readFileSync(file, "utf8"));
    for (const m of src.matchAll(/role="switch"/g)) {
      switches += 1;
      const tag = openingTag(src, m.index);
      // An aria attribute, full stop — see the note on the rule above.
      if (!/aria-label[=\s]|aria-labelledby[=\s]/.test(tag)) {
        findings.push({
          file: file.slice(root.length + 1),
          line: src.slice(0, m.index).split("\n").length,
        });
      }
    }
  }
  return { findings, switches };
}

if (SELF_TEST) {
  const dir = mkdtempSync(join(tmpdir(), "a11y-names-selftest-"));
  try {
    mkdirSync(join(dir, "src"), { recursive: true });
    writeFileSync(
      join(dir, "src/Fixture.tsx"),
      // One nameless, one named, one whose `role="switch"` is only in a
      // comment. The comment case is the false positive that made the
      // first real scan of this repo report three phantom findings.
      'export const A = () => <button role="switch" aria-checked={on} onClick={() => go()} />;\n' +
        'export const B = () => <button role="switch" aria-label="Named" aria-checked={on} />;\n' +
        '// role="switch" in a comment is not a control\n',
    );
    const { findings } = check(dir);
    if (findings.length !== 1) {
      console.error(
        `self-test FAILED: expected exactly one nameless switch, got ${JSON.stringify(findings)}`,
      );
      process.exit(1);
    }
    console.log("self-test ok — fires on a nameless switch, spares a named one and a comment");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  process.exit(0);
}

const { findings, switches } = check(process.cwd());

// Green is not evidence that anything happened: a scan that found no
// switches at all would report no findings.
if (switches < 5) {
  console.error(`a11y: refusing a vacuous pass — found only ${switches} switch(es)`);
  process.exit(1);
}

if (findings.length) {
  console.error(`a11y: ${findings.length} switch(es) with no accessible name\n`);
  for (const f of findings) console.error(`  ${f.file}:${f.line}`);
  console.error(
    "\nA <button role=\"switch\"> with no text content needs `aria-label` or\n" +
      "`aria-labelledby`. Visible text beside it is not a label. See\n" +
      "`SettingToggleRow`, which is the canonical version of this row.",
  );
  process.exit(1);
}

console.log(`a11y OK — ${switches} switches, all named`);
