// Does prose inside a flex or grid container keep its spaces?
//
// A flex (or grid) container does not lay its children out as a line of
// text. Every run of text directly inside it becomes its own anonymous
// flex item, and white space at the start and end of an item is thrown
// away. So this renders "or clickReindexto backfill":
//
//   <div style={{ display: "flex" }}>
//     <Trans i18nKey="…" components={{ em: <em /> }} />
//   </div>
//
// even though the catalog string reads "or click <em>Reindex</em> to
// backfill". `<Trans>` with `components` returns several siblings — text,
// element, text — and each one is laid out alone. Nothing reports it:
// the DOM's `textContent` still has the spaces, so a render test that
// asserts on text passes, and `tsc` has no opinion about layout.
//
// Four shipped that way, found on 2026-09-18 when a screenshot re-capture
// showed the Activities empty state; the Sept 5 capture already had it.
// Two of the four (`.envvar-note`) are flex only through a CSS class,
// which is why this reads the stylesheets as well as inline styles.
// The fix is one wrapper: a `<span>` is a single flex item, and inside it
// the text flows as a line again.
//
// ## What it flags, inside a flex or grid element
//
// 1. `<Trans components={…}>` as a direct child (fragments are
//    transparent — their children are the container's children).
// 2. JSX text that starts or ends with a space next to a sibling
//    **element**: `Enable <code>x</code> blocks`. JSX keeps a space only
//    when it shares a line with the neighbour, so a space followed by a
//    newline is not a finding.
//
// ## What it does not flag, on purpose
//
// - Text beside an **expression**: `Enable {name} blocks`. Contiguous
//   text runs are wrapped in ONE anonymous item, so the spaces between
//   them survive; only an element boundary splits the line.
// - A spacer — `{" "}` or a same-line `<Glyph /> {label}` — next to an
//   element. Flex discards it too, but nearly every such row sets a
//   `gap`, which is the spacing the reader sees. There are ~90 of them;
//   flagging dead spacers would be churn with no visible change, and a
//   gate that reports invisible things teaches people to ignore it.
//
// ## How "flex or grid" is decided
//
// - `style={{ display: … }}`, following spreads of same-file constants,
//   and either arm of a conditional whose arms are both literals.
// - A static `className` whose own rule in `src/styles/` (or App.css)
//   sets `display` — the rule's selector must be that class alone, so a
//   state rule like `.x[data-open]` or a descendant rule does not count.
//
// Anything else is unknown and passes. The check misses a flex parent it
// cannot resolve; it does not guess one.
//
//   node scripts/check-inline-flow.mjs [--self-test]

import ts from "typescript";
import { readFileSync, readdirSync, statSync, writeFileSync, mkdtempSync, mkdirSync, rmSync } from "node:fs";
import { join, sep, dirname } from "node:path";
import { tmpdir } from "node:os";

const SELF_TEST = process.argv.includes("--self-test");
const FLOWLESS = /^(flex|inline-flex|grid|inline-grid)$/;

function walk(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) walk(p, out);
    else out.push(p);
  }
  return out;
}

/** Class name → `display` value, for rules whose selector is that class alone. */
function classDisplays(root) {
  const sheets = walk(join(root, "src/styles")).filter((f) => f.endsWith(".css"));
  const appCss = join(root, "src/App.css");
  try {
    statSync(appCss);
    sheets.push(appCss);
  } catch {
    // optional — the self-test fixture has none
  }
  const out = new Map();
  for (const file of sheets) {
    const css = readFileSync(file, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
    for (const rule of css.matchAll(/([^{}@]+)\{([^{}]*)\}/g)) {
      const display = /(?:^|;|\s)display\s*:\s*([a-z-]+)/.exec(rule[2]);
      if (!display) continue;
      for (const sel of rule[1].split(",")) {
        const m = /^\s*\.([a-zA-Z][\w-]*)\s*$/.exec(sel);
        if (m) out.set(m[1], display[1]);
      }
    }
  }
  return out;
}

const attr = (opening, name) =>
  opening.attributes.properties.find(
    (a) => ts.isJsxAttribute(a) && a.name.getText() === name,
  );

/** Every `display` value an expression can produce, or [] when unknown. */
function displaysOf(expr, sf, depth = 0) {
  if (!expr || depth > 5) return [];
  if (ts.isParenthesizedExpression(expr) || ts.isAsExpression(expr) || ts.isSatisfiesExpression(expr)) {
    return displaysOf(expr.expression, sf, depth + 1);
  }
  if (ts.isStringLiteral(expr) || ts.isNoSubstitutionTemplateLiteral(expr)) return [expr.text];
  if (ts.isConditionalExpression(expr)) {
    return [
      ...displaysOf(expr.whenTrue, sf, depth + 1),
      ...displaysOf(expr.whenFalse, sf, depth + 1),
    ];
  }
  return [];
}

/** The `display` values a style object can carry. */
function styleDisplays(expr, sf, depth = 0) {
  if (!expr || depth > 5) return [];
  if (ts.isParenthesizedExpression(expr) || ts.isAsExpression(expr) || ts.isSatisfiesExpression(expr)) {
    return styleDisplays(expr.expression, sf, depth + 1);
  }
  if (ts.isIdentifier(expr)) {
    let init = null;
    const find = (n) => {
      if (init) return;
      if (ts.isVariableDeclaration(n) && ts.isIdentifier(n.name) && n.name.text === expr.text) {
        init = n.initializer ?? null;
      }
      ts.forEachChild(n, find);
    };
    find(sf);
    return styleDisplays(init, sf, depth + 1);
  }
  if (!ts.isObjectLiteralExpression(expr)) return [];
  // A later `display` overrides an earlier one, spread or not.
  let found = [];
  for (const p of expr.properties) {
    if (ts.isSpreadAssignment(p)) {
      const inner = styleDisplays(p.expression, sf, depth + 1);
      if (inner.length) found = inner;
    } else if (ts.isPropertyAssignment(p) && p.name.getText(sf) === "display") {
      found = displaysOf(p.initializer, sf, depth + 1);
    }
  }
  return found;
}

function staticClasses(opening) {
  const a = attr(opening, "className");
  if (!a || !a.initializer) return [];
  let init = a.initializer;
  if (ts.isJsxExpression(init)) init = init.expression;
  if (init && (ts.isStringLiteral(init) || ts.isNoSubstitutionTemplateLiteral(init))) {
    return init.text.split(/\s+/).filter(Boolean);
  }
  return [];
}

/** Why this element lays its children out without text flow, or null. */
function flowless(opening, sf, classes) {
  const style = attr(opening, "style");
  if (style && style.initializer && ts.isJsxExpression(style.initializer)) {
    const d = styleDisplays(style.initializer.expression, sf).find((v) => FLOWLESS.test(v));
    if (d) return `style display: ${d}`;
  }
  for (const c of staticClasses(opening)) {
    const d = classes.get(c);
    if (d && FLOWLESS.test(d)) return `.${c} is display: ${d}`;
  }
  return null;
}

/** Children as the DOM will see them: fragments flattened, empty text dropped. */
function flatChildren(children) {
  const out = [];
  for (const c of children) {
    if (ts.isJsxFragment(c)) out.push(...flatChildren(c.children));
    else if (ts.isJsxText(c) && c.containsOnlyTriviaWhiteSpaces) continue;
    else if (ts.isJsxExpression(c) && !c.expression) continue; // a comment
    else out.push(c);
  }
  return out;
}

const isTransWithComponents = (n) => {
  const opening = ts.isJsxSelfClosingElement(n) ? n : ts.isJsxElement(n) ? n.openingElement : null;
  return !!opening && opening.tagName.getText() === "Trans" && !!attr(opening, "components");
};

const isElement = (n) => !!n && (ts.isJsxElement(n) || ts.isJsxSelfClosingElement(n));

function scanFile(file, rel, classes) {
  const src = readFileSync(file, "utf8");
  const sf = ts.createSourceFile(file, src, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const findings = [];
  const report = (node, why, what) => {
    const { line } = sf.getLineAndCharacterOfPosition(node.getStart(sf));
    findings.push(`${rel}:${line + 1}  ${what} (${why})`);
  };
  const visit = (n) => {
    if (ts.isJsxElement(n)) {
      const why = flowless(n.openingElement, sf, classes);
      if (why) {
        const kids = flatChildren(n.children);
        kids.forEach((k, i) => {
          if (isTransWithComponents(k)) {
            report(k, why, "<Trans components> renders several text runs directly in it");
          } else if (ts.isJsxText(k)) {
            // JSX keeps an edge space only when no newline follows/precedes it.
            const lead = isElement(kids[i - 1]) && /^[ \t]+\S/.test(k.text);
            const trail = isElement(kids[i + 1]) && /\S[ \t]+$/.test(k.text);
            if (lead || trail) {
              report(k, why, `text ${JSON.stringify(k.text.trim().slice(0, 40))} loses the space beside an element`);
            }
          }
        });
      }
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
  return findings;
}

function check(root) {
  const classes = classDisplays(root);
  const files = walk(join(root, "src")).filter(
    (f) => f.endsWith(".tsx") && !/\.test\.tsx$/.test(f),
  );
  const findings = files.flatMap((f) => scanFile(f, f.slice(root.length + 1).split(sep).join("/"), classes));
  return { findings, files: files.length, flexClasses: [...classes.values()].filter((d) => FLOWLESS.test(d)).length };
}

function selfTest() {
  const dir = mkdtempSync(join(tmpdir(), "inline-flow-"));
  const put = (rel, body) => {
    mkdirSync(dirname(join(dir, rel)), { recursive: true });
    writeFileSync(join(dir, rel), body);
  };
  try {
    put("src/styles/a.css", `
      .row { display: flex; gap: 4px; }
      .row[data-open] { display: grid; }
      .prose { display: block; }
      .card .nested { display: flex; }
    `);
    put("src/Bad.tsx", `
      const base = { display: "flex" } as const;
      export const A = () => (
        <div style={{ display: "flex" }}>
          <Trans i18nKey="k" components={{ em: <em /> }} />
        </div>
      );
      export const B = () => <p className="row"><Trans i18nKey="k" components={{ c: <code /> }} /></p>;
      export const C = () => <label style={{ ...base, color: "red" }}>Enable <code>x</code> blocks</label>;
      export const D = (on: boolean) => <div style={{ display: on ? "grid" : "block" }}><b>a</b> and more</div>;
      export const E = () => <div style={{ display: "flex" }}><><Trans i18nKey="k" components={{ b: <b /> }} /></></div>;
    `);
    put("src/Good.tsx", `
      export const A = () => (
        <div style={{ display: "flex" }}>
          <span><Trans i18nKey="k" components={{ em: <em /> }} /></span>
        </div>
      );
      export const B = () => <p className="prose"><Trans i18nKey="k" components={{ c: <code /> }} /></p>;
      export const C = () => <p className="nested">Enable <code>x</code> blocks</p>;
      export const D = () => (
        <div style={{ display: "flex" }}>
          <Glyph />
          Plain words
        </div>
      );
      export const E = () => <div style={{ display: "flex" }}><Trans i18nKey="plain" /></div>;
      export const F = () => <div style={{ display: "flex", ...{} }}>{" "}</div>;
      export const G = () => <div style={{ display: "block" }}>Enable <code>x</code> blocks</div>;
      export const H = (name: string) => <div className="row">Enable {name} blocks</div>;
      export const I = () => <div className="row"><code>a</code>{" "}<b>b</b></div>;
    `);
    const { findings } = check(dir);
    const bad = findings.filter((f) => f.startsWith("src/Bad.tsx"));
    const good = findings.filter((f) => f.startsWith("src/Good.tsx"));
    const want = [
      /Bad\.tsx:5 .*Trans/,
      /Bad\.tsx:8 .*Trans.*\.row is display: flex/,
      /Bad\.tsx:9 .*"Enable".*loses the space/,
      /Bad\.tsx:9 .*"blocks".*loses the space/,
      /Bad\.tsx:10 .*"and more".*grid/,
      /Bad\.tsx:11 .*Trans/,
    ];
    const missing = want.filter((re) => !bad.some((f) => re.test(f)));
    if (missing.length || bad.length !== want.length || good.length) {
      console.error("check-inline-flow self-test FAILED");
      console.error("  expected but not reported:", missing.map(String));
      console.error("  reported in Bad.tsx:", bad);
      console.error("  false positives in Good.tsx:", good);
      process.exit(1);
    }
    console.log(`check-inline-flow self-test: ok — ${bad.length} planted defects reported, 0 false positives`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

if (SELF_TEST) {
  selfTest();
} else {
  const root = process.cwd();
  const { findings, files, flexClasses } = check(root);
  // A scan that read nothing passes vacuously; refuse that.
  if (files < 100 || flexClasses < 20) {
    console.error(`check-inline-flow: read only ${files} files and ${flexClasses} flex/grid classes — wrong directory?`);
    process.exit(1);
  }
  if (findings.length) {
    console.error(`check-inline-flow: ${findings.length} place(s) where text sits directly in a flex/grid container and loses its spaces.`);
    console.error("Wrap the text in a <span> so it is one flex item and flows as a line.\n");
    for (const f of findings) console.error(`  ${f}`);
    process.exit(1);
  }
  console.log(`check-inline-flow: ok — ${files} files, ${flexClasses} flex/grid classes`);
}
