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

/** Properties that mean "this draws its own field chrome". */
const CHROME_PROPS = /\b(border|background|appearance|outline|boxShadow|WebkitAppearance)/i;

/**
 * Helpers imported from elsewhere that are known to draw field chrome.
 *
 * Only for cross-file references the in-file resolver below cannot follow.
 * A NAME allowlist alone was the wrong shape: it reported `areaStyle`,
 * `TEXTAREA_STYLE` and two more as bare, all of which set a border and a
 * background in a const three lines up. Resolve first, allowlist last.
 */
const CHROME_HELPERS = ["inputStyle", "fieldControl", "fieldShell"];

/**
 * Does `name` refer to something in THIS file that sets chrome?
 *
 * Follows `const name = {…}`, `const name = () => ({…})` and
 * `function name(…) { … }` — the three shapes the renderer actually uses
 * for a shared style object.
 */
function resolvesToChrome(src, name) {
  const decl = new RegExp(
    `(?:const|let|var)\\s+${name}\\s*[=:]|function\\s+${name}\\s*\\(`,
  );
  const m = decl.exec(src);
  if (!m) return false;
  // Read the balanced block that follows, so a long style object is read
  // whole rather than to an arbitrary character count.
  let depth = 0, started = false;
  for (let i = m.index; i < src.length; i++) {
    const c = src[i];
    if (c === "{") { depth++; started = true; }
    else if (c === "}") {
      depth--;
      if (started && depth === 0) return CHROME_PROPS.test(src.slice(m.index, i + 1));
    }
  }
  return false;
}

/** Every class the library owns rather than us. */
const VENDOR = /^lucide/;

/**
 * Rules that exist with nothing rendering their class, kept on purpose.
 *
 * Validated in BOTH directions: an entry that is no longer dead, or no
 * longer defined at all, is itself a finding — so an allowlist cannot
 * outlive its reason. Same discipline as `UNSUBSCRIBED_BY_DESIGN` in
 * `verify_docs.rs`.
 *
 * Both current entries are one bug, found by the reverse scan on
 * 2026-08-29: an ancestor class nothing renders, with the whole style
 * block for its LIVE children nested under it. `CleanOrphansModal`
 * renders `clean-summary`, `clean-empty`, `clean-error` and five more,
 * but never `clean-modal` — so `.clean-modal .clean-summary` can never
 * match and those elements render unstyled. This is the third instance
 * of the family the forward check above was written for, and the
 * forward check cannot see it: it asks whether a name appears in SOME
 * selector, never whether that selector can match.
 *
 * They are kept rather than deleted because the fix is a design call —
 * de-scope the rules so the modal is styled, or drop the classNames —
 * and deleting them first would leave a red gate with no rule to
 * restore.
 */
const KNOWN_DEAD = new Map([
  // Empty, and that is the intended resting state. Two entries lived
  // here between 2026-08-29's reverse scan and its fix: `clean-modal`
  // and `modal-body`, both the same shape — an ancestor class nothing
  // renders, with the whole style block for its LIVE children nested
  // under it, so those children rendered unstyled. Both are resolved
  // (the rules were de-scoped so they can match), which is why the
  // allowlist is empty rather than deleted: the next one goes here with
  // its reason, and the both-directions check below means it cannot
  // outlive it.
]);

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

/** HTML comments hide text from the browser too — strip before scanning. */
const stripHtmlComments = (src) => src.replace(/<!--[\s\S]*?-->/g, "");

/**
 * Every static `className` value in a chunk of JSX source.
 *
 * One extractor, used by all three scans. They previously read only
 * `className="..."`, so `className={"x"}` and `className='x'` were
 * invisible: the forward scan could miss a class with no rule, and
 * `bareControls` reported a properly styled control as bare.
 */
// `\s*` around `=`: `className = "foo"` is valid JSX and was invisible.
const CLASSNAME_RE = /className\s*=\s*(?:"([^"]*)"|'([^']*)'|\{\s*(?:"([^"]*)"|'([^']*)'|`([^`]*)`)\s*\})/g;
function staticClassNames(src) {
  const out = [];
  for (const m of src.matchAll(CLASSNAME_RE)) {
    const value = m[1] ?? m[2] ?? m[3] ?? m[4] ?? m[5];
    if (value === undefined) continue;
    // A template literal with interpolation still names static classes:
    // `` `row ${active ? "sel" : ""}` `` really does put `row` on the
    // element, and dropping the whole literal loses it.
    //
    // Only WHOLE names count. A chunk touching a `${` on either side is a
    // fragment of a dynamic name, not a name: `` `phase-${s}` `` yields
    // "phase-", and the first version reported `.phase-`, `.status-` and
    // `.status-badge-` as unstyled classes. Names inside the holes are
    // skipped too — a quoted string in `${variant === "default" ? …}` is
    // a comparison value, and that version reported `.default` and
    // `.tag`. Dynamic names stay out of reach, as this file's header says.
    // An interpolation whose every string arm starts with whitespace (or is
    // empty) cannot extend the token before it: `` `statusbar-chip${c ? " warn"
    // : ""}` `` really does put `statusbar-chip` on the element. Without this
    // the conservative boundary rule dropped it — two live call sites.
    const opensCleanly = (hole) => {
      const lits = [...hole.matchAll(/(["'`])([\s\S]*?)\1/g)].map((m) => m[2]);
      return lits.length > 0 && lits.every((l) => l === "" || /^\s/.test(l));
    };
    const holes = /\$\{[^}]*\}/g;
    let cursor = 0;
    const segments = [];
    for (const h of value.matchAll(holes)) {
      segments.push({
        text: value.slice(cursor, h.index),
        openEnd: !opensCleanly(h[0]),
      });
      cursor = h.index + h[0].length;
    }
    segments.push({ text: value.slice(cursor), openEnd: false });
    segments.forEach((seg, i) => {
      const openStart = i > 0 && segments[i - 1].openEnd;
      const names = seg.text.split(/\s+/);
      names.forEach((name, j) => {
        if (!name) return;
        const touchesStart = openStart && j === 0 && !/^\s/.test(seg.text);
        const touchesEnd =
          seg.openEnd && j === names.length - 1 && !/\s$/.test(seg.text);
        if (touchesStart || touchesEnd) return; // fragment of a dynamic name
        out.push(name);
      });
    });
  }
  return out;
}

/** `.name` occurrences across every stylesheet plus the HTML shell. */
function definedClasses(root) {
  // Comments are stripped first. Without it a class named only inside a
  // comment counts as "defined" and hides a real orphan — the same trap
  // this file documents for the other direction.
  // Declaration BODIES are removed before names are read. `.foo` inside a
  // value — `content: ".foo"`, a url(), a font name — is not a definition,
  // and counting it would let a real orphan hide behind a coincidence.
  // Declaration bodies AND at-rule preludes are dropped. `@import
  // "./styles/components/base.css";` leaves `.css` behind otherwise, which
  // makes `css` a defined class — 23 such matches in App.css alone, so a
  // real orphan named `css` would have passed.
  const selectorsOnly = (text) =>
    text.replace(/\{[^{}]*\}/g, "{}").replace(/@[a-zA-Z-]+[^;{]*;/g, "");
  const sheets = walk(join(root, "src/styles")).filter((f) => f.endsWith(".css"));
  let css = sheets
    .map((f) => selectorsOnly(stripComments(readFileSync(f, "utf8"))))
    .join("\n");
  try {
    css += "\n" + selectorsOnly(stripComments(readFileSync(join(root, "src/App.css"), "utf8")));
  } catch { /* optional — the self-test fixture has none */ }
  try {
    css += "\n" + stripHtmlComments(readFileSync(join(root, "index.html"), "utf8"));
  } catch { /* optional */ }
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
    for (const name of staticClassNames(src)) {
      if (!uses.has(name)) uses.set(name, new Set());
      uses.get(name).add(file.slice(root.length + 1));
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
      const styled = staticClassNames(attrs).some((c) => defined.has(c));
      // An inline `style=` is evidence the author drew the chrome — but
      // only when it plausibly does. `style={{ color: "red" }}` sets no
      // chrome and leaves the UA border intact, which the first version
      // accepted. An OPAQUE style (an identifier or a call, e.g.
      // `AgentForm`'s `style={inputStyle(...)}`) is still accepted: its
      // contents are not readable here, and guessing would fail the
      // common shape.
      // `checkbox` / `radio` / `file` want the native control: their
      // chrome IS the UA's, and the app relies on it — `UpdatesPanel`
      // carries a note about native checkbox rendering in the Tauri
      // webview. Only text-entry controls are in scope.
      // Any static literal form, case-insensitive — HTML type values are
      // case-insensitive and `type={"checkbox"}` is the same attribute.
      const nativeByDesign =
        /type=\s*(?:["']|\{\s*["'`])\s*(checkbox|radio|file)\s*(?:["'`])/i.test(attrs);
      const styleExpr = /style=\{\{([\s\S]*?)\}\}/.exec(attrs);
      // An opaque style is chrome evidence only when it names a helper we
      // know draws chrome. `style={colorOnly}` proves nothing, and
      // accepting every opaque reference was how six unstyled inputs
      // passed. New helpers go in CHROME_HELPERS, which is checked against
      // the source so an entry cannot outlive the thing it names.
      const opaqueRef = /style=\{\s*([A-Za-z_$][\w$]*)/.exec(attrs);
      const opaqueStyle =
        opaqueRef !== null &&
        (CHROME_HELPERS.includes(opaqueRef[1]) ||
          resolvesToChrome(src, opaqueRef[1]));
      // A SPREAD inside the object literal is opaque for the same reason
      // an identifier is: `style={{ ...inputStyle(), resize: "vertical" }}`
      // is `AgentForm`'s real shape, and `inputStyle()` does set a border
      // and a background — this file just cannot see them. Requiring a
      // literal chrome keyword reported 19 correctly-styled controls.
      const drawsChrome =
        opaqueStyle ||
        (styleExpr !== null &&
          ([...styleExpr[1].matchAll(/\.\.\.\s*([A-Za-z_$][\w$]*)/g)].some(
            (r) => CHROME_HELPERS.includes(r[1]) || resolvesToChrome(src, r[1]),
          ) ||
            CHROME_PROPS.test(styleExpr[1])));
      if (!nativeByDesign && !styled && !drawsChrome) {
        findings.push({ tag: m[1], file: file.slice(root.length + 1) });
      }
    }
  }
  return findings;
}

/**
 * Every class name a rule in `src/styles` styles, mapped to its shard.
 *
 * Deliberately looser than `definedClasses`, which flattens every sheet
 * into one blob: the reverse check has to say WHICH file a dead rule is
 * in, or the report is unactionable.
 */
function definedByShard(root) {
  const sheets = walk(join(root, "src/styles")).filter((f) => f.endsWith(".css"));
  const files = [...sheets, join(root, "src/App.css")];
  const byName = new Map();
  const vendorAdjacent = new Map();
  for (const file of files) {
    let css;
    try {
      css = readFileSync(file, "utf8");
    } catch {
      continue; // Optional — the self-test fixture has no App.css.
    }
    css = css.replace(/\/\*[\s\S]*?\*\//g, "");
    // Per SELECTOR, not per file. highlight.js writes its own markup, so
    // `markdown.css` styles classes our source never names —
    // `.hljs-title.class_.inherited__` is three of them in one selector,
    // and a name-prefix test only catches the first. A class is vendor
    // when EVERY selector mentioning it also mentions hljs.
    for (const rule of css.matchAll(/([^{}]+)\{[^{}]*\}/g)) {
      const selector = rule[1];
      const vendorSel = /\.hljs/.test(selector);
      for (const m of selector.matchAll(/\.([a-zA-Z][\w-]*)/g)) {
        if (!byName.has(m[1])) byName.set(m[1], new Set());
        byName.get(m[1]).add(file.slice(root.length + 1));
        const prev = vendorAdjacent.get(m[1]);
        vendorAdjacent.set(m[1], prev === undefined ? vendorSel : prev && vendorSel);
      }
    }
  }
  return { byName, vendorAdjacent };
}

/**
 * Does this class name appear ANYWHERE in the renderer's source text?
 *
 * The reverse direction cannot reuse `usedClasses`. That reads only
 * string-literal `className` attributes, which is right for asking "is
 * this name styled?" and far too strict for asking "is this rule dead?"
 * — a name assembled as `` `row ${active ? "sel" : ""}` `` never appears
 * as a literal, and deleting its rule on that evidence would break a
 * live surface.
 *
 * So this searches the raw source with a delimiter on each side. It
 * over-reports LIVE (a name mentioned in a string that is not a class
 * still counts), which is the safe direction for a deletion tool.
 */
function referencedAnywhere(root) {
  const files = walk(join(root, "src")).filter(
    (f) => /\.(tsx|ts)$/.test(f) && !/\.test\.(tsx|ts)$/.test(f),
  );
  let blob = files.map((f) => stripComments(readFileSync(f, "utf8"))).join("\n");
  try {
    blob += "\n" + stripHtmlComments(readFileSync(join(root, "index.html"), "utf8"));
  } catch { /* optional */ }
  const memo = new Map();
  return (name) => {
    if (!memo.has(name)) {
      const esc = name.replace(/[-]/g, "\\-");
      memo.set(name, new RegExp(`[\\s"'\`.{(\\[|]${esc}(?=[\\s"'\`}\\])|,:.]|$)`).test(blob));
    }
    return memo.get(name);
  };
}

/** Class names with a rule behind them that nothing in `src/` names. */
function deadClasses(root) {
  const { byName, vendorAdjacent } = definedByShard(root);
  const isLive = referencedAnywhere(root);
  const dead = [];
  for (const [name, shards] of byName) {
    if (VENDOR.test(name) || /^hljs/.test(name)) continue;
    if (vendorAdjacent.get(name)) continue;
    if (isLive(name)) continue;
    dead.push({ name, shards: [...shards].sort() });
  }
  return dead.sort((a, b) => a.shards[0].localeCompare(b.shards[0]) || a.name.localeCompare(b.name));
}

function check(root) {
  const defined = definedClasses(root);
  const used = usedClasses(root);
  const orphans = [...used]
    .filter(([name]) => !VENDOR.test(name))
    .filter(([name]) => !defined.has(name));

  // Reverse direction: a rule nothing renders. Unlike the forward check
  // this is not a rendering bug on its own — it is dead weight, and it
  // hid a real one (see KNOWN_DEAD).
  const dead = deadClasses(root).filter((d) => !KNOWN_DEAD.has(d.name));
  const staleAllow = [];
  for (const [name, why] of KNOWN_DEAD) {
    const stillDefined = defined.has(name);
    const stillDead = deadClasses(root).some((d) => d.name === name);
    if (!stillDefined) staleAllow.push(`${name}: no longer defined — drop the allowlist entry (${why})`);
    else if (!stillDead) staleAllow.push(`${name}: is referenced now — drop the allowlist entry (${why})`);
  }
  return { defined, used, orphans, dead, staleAllow, bare: bareControls(root, defined) };
}

if (process.argv.includes("--dead-report")) {
  // Phase 3 step 1 of dev-docs/reports/ui-audit-2026-08-29.md: REPORT,
  // do not gate and do not delete. The per-shard review is a judgement
  // call about whether a shard is stale or staged for a planned surface,
  // and a tool cannot make it.
  const dead = deadClasses(process.cwd());
  const byShard = new Map();
  for (const d of dead) {
    const key = d.shards.join(", ");
    if (!byShard.has(key)) byShard.set(key, []);
    byShard.get(key).push(d.name);
  }
  console.log(`${dead.length} class name(s) with a rule and no reference in src/\n`);
  for (const [shard, names] of [...byShard].sort((a, b) => b[1].length - a[1].length)) {
    console.log(`  ${shard}  (${names.length})`);
    console.log(`    ${names.join(" ")}\n`);
  }
  process.exit(0);
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
    writeFileSync(
      join(styles, "x.css"),
      ".real { color: red }\n" +
        // Nothing renders `.ghosted` — the reverse check must say so.
        ".ghosted { color: blue }\n" +
        // A vendor class highlight.js writes itself: not ours, not dead.
        ".hljs-keyword { color: green }\n" +
        // A compound where only the modifier is rendered. It can never
        // match while `.ghosted` is unrendered, so it is dead too.
        ".ghosted.real { color: teal }\n",
    );
    writeFileSync(
      join(dir, "src/Thing.tsx"),
      'export const A = () => <div className="real invented" />;\n' +
        // Three controls: one bare (a finding), one with a styled class
        // and one with an inline style (both fine).
        'export const B = () => <input />;\n' +
        'export const C = () => <input className="real" />;\n' +
        'export const D = () => <textarea style={{ border: "none" }} />;\n',
    );
    const { orphans, dead, bare } = check(dir);
    const deadNames = dead.map((d) => d.name);
    if (!deadNames.includes("ghosted")) {
      console.error(`self-test FAILED: a dead rule was not reported (got ${deadNames})`);
      process.exit(1);
    }
    if (deadNames.includes("real")) {
      console.error("self-test FAILED: a rendered class was reported dead");
      process.exit(1);
    }
    if (deadNames.some((n) => n.startsWith("hljs"))) {
      console.error("self-test FAILED: a vendor class was reported dead");
      process.exit(1);
    }
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
      "self-test ok — fires on an invented class, a dead rule and a bare " +
        "control; spares a defined class, a vendor class, a styled control " +
        "and an inline-styled one",
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  process.exit(0);
}

const { defined, used, orphans, dead, staleAllow, bare } = check(process.cwd());

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

if (dead.length) {
  console.error(`classes: ${dead.length} rule(s) whose class nothing renders\n`);
  for (const { name, shards } of dead) console.error(`  .${name}  ${shards.join(", ")}`);
  console.error(
    "\nDead CSS ships in every bundle and hides real bugs — a whole style block\n" +
      "nested under a class nobody renders looks fine here. Delete the rule, or\n" +
      "add it to KNOWN_DEAD with the reason.",
  );
  process.exit(1);
}

if (staleAllow.length) {
  console.error(`classes: ${staleAllow.length} stale KNOWN_DEAD entr(ies)\n`);
  for (const m of staleAllow) console.error(`  ${m}`);
  console.error("\nAn allowlist entry must not outlive its reason.");
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
    `in src/styles; no dead rules (${KNOWN_DEAD.size} allowlisted); no bare form controls`,
);
