// Does the palette actually meet the contrast it promises?
//
// `design.md`'s accessibility floor says colour never carries meaning
// alone. It says nothing about whether the colour can be READ, and
// nothing measured it — so `--ok`, `--warn`, `--danger` and `--info`
// shipped at 3.48 / 2.51 / 4.22 / 3.82 against a light background,
// all below AA, while being used as `--fs-xs` body text in 20+ places.
//
// The mechanism was structural rather than a bad colour pick. Those
// four were declared once in the base `:root` and never overridden per
// theme, unlike `--fg` / `--bg` / `--accent`, so one value had to serve
// both surfaces and was tuned for the dark one. `--info` already had a
// dark override, which is what shows the pattern was known and applied
// to one of the four.
//
//   node scripts/check-contrast.mjs [--self-test]
//
// ## Why a gate and not a one-time retune
//
// A retune decays: the next person to add a semantic colour has nothing
// telling them the floor exists. This file is that floor, and its PAIRS
// table doubles as the documentation of which pairings the design
// actually promises — a question the stylesheets could not answer.
//
// ## The three things it checks
//
// 1. **Every pair in PAIRS meets its target**, in both themes.
// 2. **The two dark blocks stay in lockstep.** `[data-theme="dark"]`
//    and the `:root:not([data-theme])` inside
//    `@media (prefers-color-scheme: dark)` must declare identical
//    values — the second exists because `data-theme` is ABSENT until
//    the user touches the toggle (`useTheme` deletes it for follow-OS),
//    and `tokens.css` already carries two comments saying they must
//    agree. Nothing enforced it.
// 3. **Waivers still measure what they claim.** A `WAIVED` entry
//    carries the ratio it was granted at; if the colour moves, the
//    waiver fails rather than silently covering a new value. Same
//    both-directions discipline as `UNSUBSCRIBED_BY_DESIGN` in
//    `verify_docs.rs`.
//
// Colour maths: OKLCH → OKLab → linear sRGB → WCAG 2.1 relative
// luminance. Alpha-bearing tokens are skipped — they composite over
// whatever is behind them, so a fixed pair cannot describe them.

import { readFileSync, readdirSync, statSync, mkdtempSync, rmSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const SELF_TEST = process.argv.includes("--self-test");

/** Normal-size text. WCAG 2.1 §1.4.3. */
const AA = 4.5;

/**
 * Pairings the design promises. `on` is the background the foreground
 * is painted on; both are resolved per theme.
 */
const PAIRS = [
  ["--fg", "--bg", AA, "body text"],
  ["--fg", "--bg-raised", AA, "body text on a card"],
  ["--fg-muted", "--bg", AA, "secondary text"],
  ["--fg-muted", "--bg-raised", AA, "secondary text on a card"],
  ["--accent-ink", "--bg-raised", AA, "accent text / Button variant=accent"],
  ["--ok", "--bg-raised", AA, "success text"],
  ["--warn", "--bg-raised", AA, "warning text"],
  ["--danger", "--bg-raised", AA, "destructive text"],
  ["--info", "--bg-raised", AA, "informational text"],
  ["--fg-faint", "--bg-raised", AA, "de-emphasised meta"],
  ["--on-color", "--accent-fill", AA, "Button variant=solid"],
  ["--on-color", "--danger-fill", AA, "Button variant=solid danger"],
];

/**
 * Known-failing pairs that are NOT being changed, each with the reason
 * and the ratio it was waived at. Asserted in both directions: a waiver
 * whose colour moved is a failure, so one cannot outlive its rationale.
 */
const WAIVED = [
  {
    fg: "--fg-ghost", bg: "--bg-raised", light: 2.00, dark: 2.00,
    why: "NON-TEXT ONLY — decorative glyph fills and one dotted underline. It cannot reach AA without collapsing into --fg-faint, which would remove the step it exists to be. The waiver holds only because it is no longer used as a text colour anywhere, which `GHOST_TEXT_BAN` below enforces.",
  },
  {
    fg: "--on-color", bg: "--accent", light: 3.02, dark: 2.64,
    why: "--accent is the identity colour, not a text background. Surfaces that paint white on it use --accent-fill; this pair is kept measured so a regression back to --accent is visible.",
  },
];

const stripComments = (s) => s.replace(/\/\*[\s\S]*?\*\//g, "");

/**
 * `--fg-ghost` is the one token allowed to sit below AA, and only
 * because it is not text. That is a claim about call sites, not about
 * the palette, so it needs its own check — otherwise the waiver quietly
 * becomes a licence to write 2:1 text again.
 *
 * The distinction is mechanical: `color: "var(--fg-ghost)"` is a style
 * property (text), `color="var(--fg-ghost)"` is a JSX prop on `Glyph`
 * (a decorative SVG fill). 26 call sites were on the wrong side of it.
 */
const GHOST = "--fg-ghost";
function ghostTextUses(root) {
  const walk = (d, out = []) => {
    for (const e of readdirSync(d)) {
      const p = join(d, e);
      statSync(p).isDirectory() ? walk(p, out) : out.push(p);
    }
    return out;
  };
  let files = [];
  try {
    files = walk(join(root, "src")).filter(
      (f) => /\.(tsx|ts|css)$/.test(f) && !/\.test\./.test(f),
    );
  } catch { return []; }
  const hits = [];
  for (const f of files) {
    const src = stripComments(readFileSync(f, "utf8")).replace(/\/\/[^\n]*/g, "");
    // `color:` (style property or CSS declaration) — never `color=`.
    // Quotes are optional and may be single: CSS has none, a JS style
    // object has either. The trailing `[,)]` catches the fallback form
    // `var(--fg-ghost, red)`, which the first version missed because it
    // required the closing paren immediately.
    // The property must BE `color`, not end in it. `border-color:` and
    // `background-color:` are decorative uses and would have been
    // reported as forbidden text — a gate that fails CI on correct code.
    const re = new RegExp(
      `(?:^|[;{,\\s])color\\s*:\\s*['"\`]?var\\(\\s*${GHOST}\\s*[,)]`,
      "gm",
    );
    for (const m of src.matchAll(re)) {
      hits.push(`${f.slice(root.length + 1)}:${src.slice(0, m.index).split("\n").length}`);
    }
  }
  return hits;
}

/** OKLCH → linear sRGB. */
function oklchToLinear(L, C, H) {
  const h = (H * Math.PI) / 180;
  const a = C * Math.cos(h);
  const b = C * Math.sin(h);
  const l_ = L + 0.3963377774 * a + 0.2158037573 * b;
  const m_ = L - 0.1055613458 * a - 0.0638541728 * b;
  const s_ = L - 0.0894841775 * a - 1.2914855480 * b;
  const l = l_ ** 3, m = m_ ** 3, s = s_ ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
  ].map((v) => Math.min(1, Math.max(0, v)));
}

const luminance = ([r, g, b]) => 0.2126 * r + 0.7152 * g + 0.0722 * b;

function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/**
 * `oklch(L% C H)` → linear sRGB. Returns null for alpha forms.
 *
 * The number pattern is strict. `[\d.]+` also matches `55..0`, whose
 * `Number()` is NaN — and NaN propagates all the way to `r < target`,
 * which is FALSE for NaN. So a malformed value would have passed the
 * gate silently, the one outcome a gate must never have.
 */
const NUM = String.raw`\d+(?:\.\d+)?`;
function parseOklch(value) {
  const m = value.match(new RegExp(`^oklch\\(\\s*(${NUM})%\\s+(${NUM})\\s+(${NUM})\\s*\\)$`));
  if (!m) return null;
  const [L, C, H] = [+m[1] / 100, +m[2], +m[3]];
  if (![L, C, H].every(Number.isFinite)) return null;
  return oklchToLinear(L, C, H);
}

const declsIn = (block) => {
  const out = {};
  // Whitespace is collapsed before the value is stored. Multi-line
  // declarations (`--shadow-modal` wraps across two lines) are indented
  // differently inside `[data-theme="dark"]` and the more-nested
  // `:root:not([data-theme])`, so a raw string compare reported the two
  // dark blocks out of lockstep over indentation alone — the gate's
  // first real run said exactly that.
  for (const m of block.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    out[m[1]] = m[2].replace(/\s+/g, " ").trim();
  }
  return out;
};

/**
 * Walk the top level, plus the one at-rule that carries a base palette
 * (`@media (prefers-color-scheme: dark)`), and merge EVERY matching
 * block in source order.
 *
 * Reading only the first match was wrong in a way that could not fail
 * loudly: `tokens.css` has three more top-level `:root` blocks after the
 * palette (the derived aliases), and a checked token redeclared in one
 * of them would win in the browser while this gate went on measuring the
 * first value. None currently is — this closes the gap, it does not fix
 * a live bug.
 *
 * `@media (prefers-contrast: more)` is deliberately NOT merged: it is a
 * conditional override, not the default palette, and folding it in would
 * measure a surface most users never see.
 */
function collectBlocks(src) {
  const ordered = [];          // every palette block, in document order
  const classify = (sel) => {
    const t = sel.replace(/\s+/g, "");
    if (t === ":root") return "root";
    if (t === ":root,[data-theme=\"light\"]") return "light";
    if (t === "[data-theme=\"dark\"]") return "dark";
    if (t === ":root:not([data-theme])") return "sys";
    return null;
  };
  const walkLevel = (text, inSystemDark) => {
    let i = 0;
    while (i < text.length) {
      const open = text.indexOf("{", i);
      if (open < 0) break;
      const sel = text.slice(i, open).replace(/\/\*[\s\S]*?\*\//g, "").trim();
      let depth = 0, j = open;
      for (; j < text.length; j++) {
        if (text[j] === "{") depth++;
        else if (text[j] === "}") { depth--; if (!depth) break; }
      }
      const body = text.slice(open + 1, j);
      if (sel.startsWith("@")) {
        if (/^@media\s*\(prefers-color-scheme:\s*dark\)/.test(sel)) walkLevel(body, true);
      } else {
        const kind = classify(sel);
        if (kind && (!inSystemDark || kind === "sys")) {
          ordered.push({ kind, decls: declsIn(body) });
        }
      }
      i = j + 1;
    }
  };
  walkLevel(src, false);
  return ordered;
}

/**
 * Resolve one theme by replaying the blocks in DOCUMENT ORDER, applying
 * every block whose selector would match that document.
 *
 * Merging bucket-by-bucket (`{...base, ...light, ...dark}`) is not the
 * cascade. All of these selectors have the same specificity, so order
 * decides — and `tokens.css` has three more `:root` blocks AFTER the
 * light palette. A token redeclared in one of them wins in the browser
 * and lost under a fixed bucket merge. No checked token is in those
 * blocks today; this removes the trap rather than fixing a live bug.
 *
 * `:root, [data-theme="light"]` applies to a DARK document too — its
 * `:root` half matches any root element — which is exactly why
 * `[data-theme="dark"]` has to come after it in the file.
 */
function resolveTheme(ordered, theme) {
  const applies = theme === "light"
    ? new Set(["root", "light"])
    : new Set(["root", "light", "dark"]);
  const out = {};
  for (const b of ordered) if (applies.has(b.kind)) Object.assign(out, b.decls);
  return out;
}

function readThemes(cssPath) {
  const src = stripComments(readFileSync(cssPath, "utf8"));
  const ordered = collectBlocks(src);
  const merge = (kind) => {
    const out = {};
    for (const b of ordered) if (b.kind === kind) Object.assign(out, b.decls);
    return out;
  };
  return {
    themes: { light: resolveTheme(ordered, "light"), dark: resolveTheme(ordered, "dark") },
    dark: merge("dark"),
    sys: merge("sys"),
  };
}

function check(root) {
  const findings = [];
  const { themes, dark, sys } = readThemes(join(root, "src", "styles", "tokens.css"));

  const resolve = (theme, token) => {
    const raw = themes[theme][token];
    if (raw === undefined) return { err: "not declared" };
    const lin = parseOklch(raw);
    if (!lin) return { err: `not a plain oklch() value (${raw})` };
    return { lin };
  };

  let measured = 0;
  for (const [fg, bg, target, label] of PAIRS) {
    for (const theme of ["light", "dark"]) {
      const f = resolve(theme, fg), b = resolve(theme, bg);
      if (f.err || b.err) {
        findings.push(`${theme}: ${f.err ? fg : bg} — ${f.err ?? b.err}`);
        continue;
      }
      const r = contrast(f.lin, b.lin);
      measured++;
      if (r < target) {
        findings.push(
          `${theme}: ${fg} on ${bg} = ${r.toFixed(2)}:1, needs ${target} — ${label}`,
        );
      }
    }
  }

  // Waivers must still measure what they were granted at.
  for (const w of WAIVED) {
    for (const theme of ["light", "dark"]) {
      const f = resolve(theme, w.fg), b = resolve(theme, w.bg);
      if (f.err || b.err) {
        findings.push(`waiver ${w.fg}/${w.bg} (${theme}): ${f.err ?? b.err}`);
        continue;
      }
      const r = contrast(f.lin, b.lin);
      measured++;
      if (Math.abs(r - w[theme]) > 0.05) {
        findings.push(
          `waiver ${w.fg} on ${w.bg} (${theme}) was granted at ${w[theme]}:1 but now measures ` +
            `${r.toFixed(2)}:1 — re-decide it, do not update the number blindly`,
        );
      }
    }
  }

  // The avatar ink is white over a hue-derived fill, so no fixed pair can
  // describe it: `avatarColorFor` builds
  // `oklch(var(--avatar-derived-l) var(--avatar-derived-c) <hue>)` from a
  // hash of the email. Sweep the wheel and hold the WORST hue to AA —
  // checking one hue would pass while a third of the accounts failed,
  // which is what L=62% was doing (3.38:1 at hue 190).
  //
  // Every integer hue, not every fifth: the sweep is 360 cheap
  // multiplications and a coarse step can straddle a failing hue.
  // Both themes, because the tokens are overridable per theme even
  // though neither currently is.
  // CSS number grammar, not a hand-narrowed one: `.12`, `+52%` and
  // exponent forms are all valid and were being rejected as malformed.
  const CSSNUM = String.raw`[-+]?(?:\d+\.\d+|\d+|\.\d+)(?:e[-+]?\d+)?`;
  const PCT = new RegExp(`^(${CSSNUM})%$`, "i");
  const NUMBER = new RegExp(`^(${CSSNUM})$`, "i");
  for (const theme of ["light", "dark"]) {
    const rawL = themes[theme]["--avatar-derived-l"];
    const rawC = themes[theme]["--avatar-derived-c"];
    if (rawL === undefined || rawC === undefined) {
      findings.push(`${theme}: --avatar-derived-l/--avatar-derived-c not declared`);
      continue;
    }
    // Strict, because `parseFloat("52px")` is 52 and would measure a
    // value the browser rejects.
    const mL = PCT.exec(rawL.trim());
    const mC = NUMBER.exec(rawC.trim());
    if (!mL || !mC) {
      findings.push(
        `${theme}: --avatar-derived-l/-c must be "<n>%" and "<n>" — got ${JSON.stringify(rawL)} / ${JSON.stringify(rawC)}`,
      );
      continue;
    }
    // The ink is whatever `--on-color` resolves to, not an assumed white:
    // change that token and this sweep must follow it.
    const ink = resolve(theme, "--on-color");
    if (ink.err) {
      findings.push(`${theme}: --on-color — ${ink.err}`);
      continue;
    }
    let worst = Infinity, worstHue = 0;
    for (let hue = 0; hue < 360; hue += 1) {
      const r = contrast(ink.lin, oklchToLinear(+mL[1] / 100, +mC[1], hue));
      if (r < worst) { worst = r; worstHue = hue; }
      measured++;
    }
    if (worst < AA) {
      findings.push(
        `${theme}: --on-color on a derived avatar fill = ${worst.toFixed(2)}:1 at hue ${worstHue}, ` +
          `needs ${AA} — lower --avatar-derived-l`,
      );
    }
  }

  // `--fg-ghost` is waived only while it is not text.
  for (const site of ghostTextUses(root)) {
    findings.push(
      `${GHOST} used as a TEXT colour at ${site} — it measures 2.00:1. ` +
        `Use --fg-faint for text; --fg-ghost is for decorative glyph fills only.`,
    );
  }

  // The two dark blocks must agree.
  for (const key of new Set([...Object.keys(dark), ...Object.keys(sys)])) {
    if (dark[key] !== sys[key]) {
      findings.push(
        `dark blocks out of lockstep: ${key} is ${dark[key] ?? "absent"} in ` +
          `[data-theme="dark"] but ${sys[key] ?? "absent"} in :root:not([data-theme])`,
      );
    }
  }

  return { findings, measured };
}

if (SELF_TEST) {
  const dir = mkdtempSync(join(tmpdir(), "contrast-selftest-"));
  try {
    mkdirSync(join(dir, "src", "styles"), { recursive: true });
    writeFileSync(
      join(dir, "src", "styles", "tokens.css"),
      `:root {\n` +
        `  --on-color: oklch(100% 0 0);\n` +
        `  --fg: oklch(22% 0.008 60);\n  --fg-muted: oklch(50% 0.008 60);\n` +
        `  --fg-faint: oklch(65% 0.006 60);\n  --fg-ghost: oklch(78% 0.004 60);\n` +
        `  --accent: oklch(68% 0.13 45);\n  --accent-ink: oklch(42% 0.10 45);\n` +
        `  --accent-fill: oklch(58% 0.13 45);\n  --danger-fill: oklch(58% 0.14 25);\n` +
        `  --ok: oklch(55% 0.10 150);\n` +
        `  --warn: oklch(72% 0.12 80);\n` + // ← too light: must fail
        `  --danger: oklch(58% 0.14 25);\n  --info: oklch(56% 0.10 220);\n` +
        `  --bg: oklch(99% 0.003 60);\n  --bg-raised: oklch(100% 0 0);\n}\n` +
        `[data-theme="dark"] {\n  --bg: oklch(16% 0.006 60);\n  --bg-raised: oklch(19% 0.006 60);\n` +
        `  --fg: oklch(92% 0.006 60);\n  --fg-muted: oklch(70% 0.008 60);\n` +
        `  --fg-faint: oklch(55% 0.008 60);\n  --fg-ghost: oklch(40% 0.006 60);\n` +
        `  --accent: oklch(72% 0.17 45);\n  --accent-ink: oklch(82% 0.14 45);\n` +
        `  --accent-fill: oklch(58% 0.17 45);\n` +
        `  --ok: oklch(62% 0.10 150);\n  --warn: oklch(72% 0.12 80);\n` +
        `  --danger: oklch(61% 0.14 25);\n  --info: oklch(72% 0.10 220);\n}\n` +
        `@media (prefers-color-scheme: dark) {\n  :root:not([data-theme]) {\n` +
        `    --bg: oklch(16% 0.006 60);\n    --bg-raised: oklch(19% 0.006 60);\n` +
        `    --fg: oklch(92% 0.006 60);\n    --fg-muted: oklch(70% 0.008 60);\n` +
        `    --fg-faint: oklch(55% 0.008 60);\n    --fg-ghost: oklch(40% 0.006 60);\n` +
        `    --accent: oklch(72% 0.17 45);\n    --accent-ink: oklch(82% 0.14 45);\n` +
        `    --accent-fill: oklch(58% 0.17 45);\n` +
        `    --ok: oklch(62% 0.10 150);\n    --warn: oklch(72% 0.12 80);\n` +
        `    --danger: oklch(99% 0.14 25);\n` + // ← lockstep break: must fail
        `    --info: oklch(72% 0.10 220);\n  }\n}\n`,
    );
    mkdirSync(join(dir, "src", "sections"), { recursive: true });
    writeFileSync(
      join(dir, "src", "sections", "Bad.tsx"),
      'export const A = () => <div style={{ color: "var(--fg-ghost)" }} />;\n' +
        // A decorative Glyph prop on the next line must NOT be reported.
        'export const B = () => <Glyph color="var(--fg-ghost)" />;\n',
    );
    const { findings } = check(dir);
    const sawGhost = findings.filter((f) => f.includes("TEXT colour"));
    if (sawGhost.length !== 1) {
      console.error(
        `self-test FAILED: expected exactly one --fg-ghost text use, got ${JSON.stringify(sawGhost)}`,
      );
      process.exit(1);
    }
    const sawWarn = findings.some((f) => f.startsWith("light: --warn"));
    const sawLockstep = findings.some((f) => f.includes("out of lockstep: --danger"));
    // --danger-fill is absent from the dark blocks on purpose: it should
    // fall through to the base value, not be reported missing.
    const falseMissing = findings.some((f) => f.includes("--danger-fill — not declared"));
    if (!sawWarn || !sawLockstep || falseMissing) {
      console.error(
        `self-test FAILED: warn=${sawWarn} lockstep=${sawLockstep} falseMissing=${falseMissing}\n` +
          findings.map((f) => "  " + f).join("\n"),
      );
      process.exit(1);
    }
    console.log(
      "self-test ok — fires on a sub-AA pair, on dark blocks out of lockstep and " +
        "on --fg-ghost used as text; spares a decorative Glyph prop and inherits " +
        "base values rather than reporting them missing",
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  process.exit(0);
}

const { findings, measured } = check(process.cwd());

// Green is not evidence that anything happened: a parse that produced an
// empty palette would measure nothing and report no findings.
if (measured < 20) {
  console.error(`contrast: refusing a vacuous pass — only ${measured} pair(s) measured`);
  process.exit(1);
}

if (findings.length) {
  console.error(`contrast: ${findings.length} finding(s)\n`);
  for (const f of findings) console.error(`  ${f}`);
  console.error(
    "\nSemantic colours are declared LIGHT in the base `:root` and overridden in\n" +
      "BOTH dark blocks. Omitting one hands dark the light value. See the note\n" +
      "above the semantic block in tokens.css.",
  );
  process.exit(1);
}

console.log(`contrast OK — ${measured} pairs measured, all meet target; dark blocks in lockstep`);
