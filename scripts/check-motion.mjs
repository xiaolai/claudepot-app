// Does `prefers-reduced-motion` actually reach the whole UI?
//
// `design.md`'s accessibility floor commits to honouring it. Six CSS
// shards carry a `@media (prefers-reduced-motion: reduce)` block, and
// between them they could never cover the primitives: those animate
// from INLINE styles, and an inline declaration beats any stylesheet
// rule that carries no `!important`. So `Button`, `IconButton`,
// `SidebarItem`, `FilterChip`, `modalParts` and the six settings
// toggles animated for every user whatever the system setting said —
// 23 inline declarations no media query in this repo could reach.
// Meanwhile `accounts.css` spent a reduced-motion block on
// `.collapsible-chevron`, a class nothing renders.
//
// The fix zeroes the duration TOKENS, because a `var()` in an inline
// style still resolves against the cascade. That makes two things
// load-bearing, and both are checked here:
//
//   1. The override exists and sits AFTER the base `--dur-*`
//      declarations. A media query adds no specificity, so at equal
//      specificity source order decides — the same trap `tokens.css`
//      already records for `--focus-ring`, whose first draft sat above
//      the declaration and was inert.
//   2. No inline `transition:` / `animation:` names a literal
//      duration. A hardcoded `120ms` steps straight back outside the
//      override's reach with nothing to say so, and `EventsSection`
//      had exactly one.
//
//   node scripts/check-motion.mjs [--self-test]
//
// The rendered-DOM half of this contract lives in
// `src/components/primitives/reducedMotion.test.tsx`: this gate reads
// source text and covers every call site, that test mounts the four
// primitives and asserts the token actually reaches `style.transition`.
// Split because reading a file needs Node and the app's tsconfig
// carries no node types — and because `?raw` yields an empty string
// under Vitest, measured, which is how an earlier CSS assertion in this
// repo passed while reading nothing.
//
// Comments are stripped from both inputs first. The doc comment above
// the override quotes `@media (prefers-reduced-motion: reduce)`
// verbatim, so a raw search finds the COMMENT and reports order that
// no browser sees. `check-classes.mjs` records the same trap.

import { readFileSync, readdirSync, statSync, mkdtempSync, rmSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const SELF_TEST = process.argv.includes("--self-test");

/** Durations that drive one-shot transitions and must be zeroed. */
const ZEROED = ["--dur-fast", "--dur-base", "--dur-slow", "--dur-hover"];

/** Looping cadence. Zeroing it freezes a spinner rather than removing
 *  the motion; the shards that own those stop them outright instead. */
const LOOPING = "--dur-pulse";

const stripComments = (s) => s.replace(/\/\*[\s\S]*?\*\//g, "");

/**
 * Remove `//` comments while tracking string state.
 *
 * The predecessor-character heuristic this replaces (`[^:"'\`]//`) only
 * spared a `//` directly after a quote — a `//` anywhere later inside a
 * string still ate the rest of the line, taking any transition on it.
 * One pass, three states, no parser.
 */
function stripLineComments(src) {
  let out = "", quote = null;
  for (let i = 0; i < src.length; i++) {
    const c = src[i];
    if (quote) {
      out += c;
      if (c === "\\") { out += src[++i] ?? ""; continue; }
      if (c === quote) quote = null;
      continue;
    }
    if (c === '"' || c === "'" || c === "`") { quote = c; out += c; continue; }
    if (c === "/" && src[i + 1] === "/") {
      while (i < src.length && src[i] !== "\n") i++;
      out += "\n";
      continue;
    }
    out += c;
  }
  return out;
}

function walk(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) walk(p, out);
    else out.push(p);
  }
  return out;
}

function check(root) {
  const findings = [];
  const tokensPath = join(root, "src", "styles", "tokens.css");
  let tokens = "";
  try {
    tokens = stripComments(readFileSync(tokensPath, "utf8"));
  } catch {
    findings.push({ file: "src/styles/tokens.css", line: 0, msg: "unreadable" });
  }

  const rule = tokens.match(/@media \(prefers-reduced-motion: reduce\)\s*\{\s*:root\s*\{([\s\S]*?)\}/);
  if (!rule) {
    findings.push({
      file: "src/styles/tokens.css",
      line: 0,
      msg: "no `@media (prefers-reduced-motion: reduce) { :root { … } }` block",
    });
  } else {
    for (const token of ZEROED) {
      // `\s*:` and a value boundary: `--dur-fast : 0ms` is valid CSS and
      // was missed, and `0msx` was accepted as `0ms`.
      if (!new RegExp(`${token}\\s*:\\s*0m?s\\s*(?:;|$)`, "m").test(rule[1])) {
        findings.push({
          file: "src/styles/tokens.css",
          line: 0,
          msg: `${token} is not zeroed under prefers-reduced-motion`,
        });
      }
    }
    if (rule[1].includes(LOOPING)) {
      findings.push({
        file: "src/styles/tokens.css",
        line: 0,
        msg: `${LOOPING} must NOT be zeroed — that freezes a looping animation instead of removing it`,
      });
    }
    // Checked per token, against this token's own declarations — not
    // against `--dur-fast` alone. An override sitting after `--dur-fast`
    // but before `--dur-base` would otherwise pass while `--dur-base`
    // still won on source order.
    const overrideAt = tokens.indexOf(rule[0]);
    const overrideEnd = overrideAt + rule[0].length;
    for (const token of ZEROED) {
      const decl = new RegExp(`${token}\\s*:`, "g");
      const positions = [...tokens.matchAll(decl)].map((m) => m.index);
      const before = positions.filter((i) => i < overrideAt).pop() ?? -1;
      const after = positions.find((i) => i >= overrideEnd) ?? -1;
      if (before === -1) {
        findings.push({
          file: "src/styles/tokens.css",
          line: 0,
          msg: `${token} has no base declaration before the reduced-motion override, so the override has nothing to beat and does nothing`,
        });
      } else if (after > -1) {
        findings.push({
          file: "src/styles/tokens.css",
          line: 0,
          msg: `${token} is re-declared AFTER the reduced-motion override, which therefore loses on source order`,
        });
      }
    }
  }

  // Inline animation declarations naming a literal duration.
  let inlineSites = 0;
  const srcDir = join(root, "src");
  let files = [];
  try {
    files = walk(srcDir).filter((f) => /\.tsx?$/.test(f) && !/\.test\./.test(f));
  } catch { /* self-test fixtures may omit src/ */ }
  for (const f of files) {
    const raw = readFileSync(f, "utf8");
    const body = stripLineComments(stripComments(raw));
    // Longhands count too, and the string need not start the value: a
    // ternary like `animation: on ? "spin 1.2s linear" : undefined` hides
    // a literal from a quote-anchored scan.
    //
    // The value is read by a SCANNER, not a line regex. A style object is
    // often one line — `{ transition: "…", background: "…" }` — and a
    // regex that runs to end-of-line swallows the next property, so an
    // unrelated literal would be attributed to `transition`. The scanner
    // stops at the first top-level `,` or `}`.
    const re = /\b(transition|animation|transitionDuration|animationDuration)\s*:\s*/g;
    let m;
    while ((m = re.exec(body))) {
      const value = (() => {
        let depth = 0, q = null, out = "";
        for (let i = m.index + m[0].length; i < body.length; i++) {
          const c = body[i];
          if (q) { out += c; if (c === q && body[i - 1] !== "\\") q = null; continue; }
          if (c === '"' || c === "'" || c === "`") { q = c; out += c; continue; }
          if (c === "{" || c === "[" || c === "(") depth++;
          else if (c === "}" || c === "]" || c === ")") { if (!depth) break; depth--; }
          else if ((c === "," || c === ";") && depth === 0) break;
          out += c;
        }
        return out;
      })();
      const literals = [...value.matchAll(/(["'`])([\s\S]*?)\1/g)].map((x) => x[2]);
      if (!literals.length) continue;
      inlineSites++;
      const scanned = literals.join(" ");
      // A literal duration is any number followed by s/ms that is not
      // part of a var() reference.
      // Checked on the literals AND on the raw value expression, because
      // `` `${120}ms` `` and `"opacity " + 120 + "ms"` put the number and
      // the unit in different tokens — neither is a literal containing a
      // duration, and both animate for 120ms.
      const rawValue = value.replace(/var\([^)]*\)/g, "");
      const hidden = /\d[\s}"'`+]*m?s\b/.test(rawValue);
      if (hidden || /(^|[\s,(])\d*\.?\d+\s*m?s\b/.test(scanned.replace(/var\([^)]*\)/g, ""))) {
        findings.push({
          file: f.slice(root.length + 1),
          line: body.slice(0, m.index).split("\n").length,
          msg: `literal duration in an inline style — outside the token override's reach: ${JSON.stringify(scanned.slice(0, 60))}`,
        });
      }
    }
  }
  // CSS shards animate too, and this gate only ever looked at the inline
  // half — so a new `animation: spin 800ms` in a stylesheet, with nothing
  // neutralising it, passed. Stylesheet motion IS reachable by a media
  // query, so the bar is different from the inline bar: a literal
  // duration is fine PROVIDED the shard carries a reduced-motion block.
  // A token-driven duration needs nothing, because zeroing the token
  // already covers it.
  let cssSites = 0;
  let sheets = [];
  try {
    sheets = walk(join(root, "src", "styles")).filter((f) => f.endsWith(".css"));
  } catch { /* self-test fixtures may have none */ }
  for (const f of sheets) {
    const css = stripComments(readFileSync(f, "utf8"));
    // Which selectors the shard actually neutralises. File-wide was too
    // coarse: one reduce block anywhere let every literal animation in
    // the shard pass, including selectors the block never mentions.
    const guardedSelectors = new Set();
    for (const blk of css.matchAll(/@media[^{]*prefers-reduced-motion[^{]*\{([\s\S]*?)\n\}/g)) {
      for (const rule of blk[1].matchAll(/([^{}]+)\{[^{}]*\}/g)) {
        for (const cls of rule[1].matchAll(/\.([a-zA-Z][\w-]*)/g)) guardedSelectors.add(cls[1]);
      }
    }
    for (const m of css.matchAll(
      /\b(animation|transition|animation-duration|transition-duration)\s*:\s*([^;}]+)/g,
    )) {
      const value = m[2];
      if (/^\s*(none|unset|initial|inherit)\b/.test(value)) continue;
      cssSites++;
      const literal = /(^|[\s,(])\d*\.?\d+\s*m?s\b/.test(value.replace(/var\([^)]*\)/g, ""));
      // The rule's own selector — scan backwards to the `{` that opened it.
      const openIdx = css.lastIndexOf("{", m.index);
      const selStart = Math.max(css.lastIndexOf("}", openIdx), css.lastIndexOf("{", openIdx - 1)) + 1;
      const ownClasses = [
        ...css.slice(selStart, openIdx).matchAll(/\.([a-zA-Z][\w-]*)/g),
      ].map((x) => x[1]);
      const covered = ownClasses.length > 0 && ownClasses.some((c) => guardedSelectors.has(c));
      if (literal && !covered) {
        findings.push({
          file: f.slice(root.length + 1),
          line: css.slice(0, m.index).split("\n").length,
          msg: `stylesheet ${m[1]} with a literal duration whose selector ` +
            `(${ownClasses.map((c) => "." + c).join("") || "?"}) is not named in any ` +
            `@media (prefers-reduced-motion: reduce) block in this shard: ` +
            JSON.stringify(value.trim().slice(0, 50)),
        });
      }
    }
  }

  return { findings, inlineSites, cssSites };
}

if (SELF_TEST) {
  const dir = mkdtempSync(join(tmpdir(), "motion-selftest-"));
  try {
    mkdirSync(join(dir, "src", "styles"), { recursive: true });
    // tokens.css: override present but placed ABOVE the declarations,
    // and missing --dur-hover. Comment quotes the at-rule to prove the
    // comment strip works.
    writeFileSync(
      join(dir, "src", "styles", "tokens.css"),
      `/* mentions @media (prefers-reduced-motion: reduce) in prose */\n` +
        `@media (prefers-reduced-motion: reduce) {\n  :root {\n    --dur-fast: 0ms;\n    --dur-base: 0ms;\n    --dur-slow: 0ms;\n  }\n}\n` +
        `:root {\n  --dur-fast: 80ms;\n  --dur-base: 120ms;\n}\n`,
    );
    // A stylesheet animation with a literal duration in a shard carrying
    // no reduce block — the half the gate was blind to until 2026-08-29.
    writeFileSync(
      join(dir, "src", "styles", "motion.css"),
      ".spinner { animation: spin 800ms linear infinite; }\n" +
        // Token-driven: covered by zeroing the token, must NOT fire.
        ".fade { transition: opacity var(--dur-base) linear; }\n",
    );
    writeFileSync(
      join(dir, "src", "Bad.tsx"),
      `export const X = () => <div style={{ transition: "background 120ms" }} />;\n` +
        `// transition: "background 999ms" in a comment must be ignored\n`,
    );
    const { findings } = check(dir);
    const kinds = findings.map((f) => f.msg);
    const wantsHover = kinds.some((k) => k.includes("--dur-hover"));
    // Matched on the token plus "override" rather than an exact phrase:
    // the first version pinned the wording, and rewording the message
    // broke the self-test while the gate itself was firing correctly.
    const wantsOrder = kinds.some((k) => k.includes("--dur-fast") && k.includes("override"));
    const wantsLiteral = kinds.some((k) => k.includes("literal duration"));
    const wantsCss = kinds.some((k) => k.includes("stylesheet animation"));
    const cssFalsePositive = kinds.some((k) => k.includes("--dur-base) linear"));
    const commentLeaked = kinds.some((k) => k.includes("999ms"));
    if (!wantsHover || !wantsOrder || !wantsLiteral || commentLeaked || !wantsCss || cssFalsePositive) {
      console.error(
        `self-test FAILED: hover=${wantsHover} order=${wantsOrder} literal=${wantsLiteral} ` +
          `commentLeaked=${commentLeaked} css=${wantsCss} cssFalsePositive=${cssFalsePositive}\n` +
          JSON.stringify(findings, null, 2),
      );
      process.exit(1);
    }
    console.log(
      "self-test ok — fires on a missing token, a mis-ordered override, a literal\n" +
        "  duration inline and an unguarded stylesheet animation; ignores a commented\n" +
        "  one and a token-driven stylesheet transition",
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  process.exit(0);
}

const { findings, inlineSites, cssSites } = check(process.cwd());

// Green is not evidence that anything happened: a scan that walked no
// files would report no findings.
if (inlineSites < 10) {
  console.error(`motion: refusing a vacuous pass — found only ${inlineSites} inline animation declaration(s)`);
  process.exit(1);
}

if (findings.length) {
  console.error(`motion: ${findings.length} finding(s)\n`);
  for (const f of findings) console.error(`  ${f.file}${f.line ? ":" + f.line : ""}  ${f.msg}`);
  console.error(
    "\nInline styles beat stylesheet rules, so a component that animates from\n" +
      "`style={{ transition: … }}` is only reachable by zeroing the duration\n" +
      "TOKENS in tokens.css. Name `var(--dur-…)`, never a literal.",
  );
  process.exit(1);
}

console.log(
  `motion OK — ${inlineSites} inline + ${cssSites} stylesheet animation declarations, ` +
    `all token-driven or reduce-guarded; reduced-motion override in place`,
);
