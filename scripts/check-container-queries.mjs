// Can every `@container` rule in the stylesheets actually apply?
//
// A container query matches against an ANCESTOR container, so a rule
// inside `@container x` that selects the element declaring `container: x`
// never applies. Nothing reports that: the CSS is valid, the build keeps
// it, `check:classes` sees a class with a rule, and every other rule in
// the block still fires. `.events-stream-panel { flex-direction: column }`
// shipped into a working tree that way: below the breakpoint the filter
// rail changed shape while its parent stayed a row, and the 1366px rail
// pushed the card stream out of the window. It took measuring the running
// app to see it.
//
// A query naming a container nothing declares fails the same silent way,
// and so does a query whose property is also set in the target element's
// inline style, because an inline declaration beats any stylesheet rule.
// Both are checked here too.
//
//   node scripts/check-container-queries.mjs [--self-test]

import { readFileSync, readdirSync, statSync, mkdtempSync, rmSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const SELF_TEST = process.argv.includes("--self-test");

const stripComments = (s) => s.replace(/\/\*[\s\S]*?\*\//g, "");

function walk(dir, out = [], ext = ".css") {
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) walk(p, out, ext);
    else if (p.endsWith(ext)) out.push(p);
  }
  return out;
}

/** The text between the `{` at `open` and its matching `}`. */
function blockBody(css, open) {
  let depth = 0;
  for (let i = open; i < css.length; i++) {
    if (css[i] === "{") depth++;
    else if (css[i] === "}" && --depth === 0) return css.slice(open + 1, i);
  }
  return css.slice(open + 1);
}

/** Classes that declare each container name, across every sheet. */
const ANONYMOUS = "(anonymous)";

function collectContainers(sheets) {
  const owners = new Map();
  for (const { css } of sheets) {
    for (const rule of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
      for (const decl of rule[2].matchAll(/(?:^|[\s;])container(?:-name)?\s*:\s*([^;}]+)/g)) {
        const names = decl[1].split("/")[0].trim().split(/\s+/).filter(Boolean);
        const classes = [...rule[1].matchAll(/\.([a-zA-Z][\w-]*)/g)].map((m) => m[1]);
        for (const name of names) {
          if (!owners.has(name)) owners.set(name, new Set());
          for (const c of classes) owners.get(name).add(c);
        }
      }
      // `container-type` alone declares a container with no name, which only
      // an unnamed query can match.
      if (/(?:^|[\s;])container-type\s*:/.test(rule[2])) {
        if (!owners.has(ANONYMOUS)) owners.set(ANONYMOUS, new Set());
        for (const m of rule[1].matchAll(/\.([a-zA-Z][\w-]*)/g)) owners.get(ANONYMOUS).add(m[1]);
      }
    }
  }
  return owners;
}

const OPAQUE = Symbol("unreadable inline style");

const camel = (prop) => prop.replace(/-([a-z])/g, (_, c) => c.toUpperCase());

const SIDED = /^(border|padding|margin)(Top|Right|Bottom|Left)$/;

/** `prop` and the inline names that would set it too (`border` for `borderRight`). */
function overlapping(prop) {
  if (SIDED.test(prop)) return [prop, prop.match(SIDED)[1]];
  if (["border", "padding", "margin"].includes(prop)) {
    return [prop, ...["Top", "Right", "Bottom", "Left"].map((side) => prop + side)];
  }
  if (prop === "flexDirection" || prop === "flexWrap") return [prop, "flexFlow"];
  if (prop === "flexFlow") return [prop, "flexDirection", "flexWrap"];
  return [prop];
}

/** The `style={{ … }}` body of the opening tag around `at`, or "". */
function inlineStyleAt(source, at) {
  const start = source.lastIndexOf("<", at);
  let depth = 0;
  let end = at;
  for (; end < source.length; end++) {
    const c = source[end];
    if (c === "{") depth++;
    else if (c === "}") depth--;
    else if (c === ">" && depth === 0 && source[end - 1] !== "=") break;
  }
  const tag = source.slice(start, end);
  // A variable or a spread can hold anything; report it as unreadable.
  if (/style=\{(?!\{)/.test(tag)) return OPAQUE;
  const body = tag.match(/style=\{\{([\s\S]*?)\}\}/)?.[1] ?? "";
  return body.includes("...") ? OPAQUE : body;
}

/** Every inline style on an element carrying `cls`. */
function inlineStylesFor(sources, cls) {
  const hits = [];
  const pattern = new RegExp(`className=(?:"[^"]*|\\{\`[^\`]*)(?<![\\w-])${cls}(?![\\w-])`, "g");
  for (const { file, text } of sources) {
    for (const m of text.matchAll(pattern)) {
      hits.push({ file, line: text.slice(0, m.index).split("\n").length, style: inlineStyleAt(text, m.index) });
    }
  }
  return hits;
}

function check(sheets, sources = []) {
  const findings = [];
  const owners = collectContainers(sheets);
  let queries = 0;
  for (const { file, css } of sheets) {
    for (const q of css.matchAll(/@container\s*(?:([a-zA-Z][\w-]*)\s*)?[^{]*\{/g)) {
      queries++;
      // Unnamed, the query matches whichever container is nearest, so a
      // rule on any declaring element is suspect.
      const name = q[1] ?? "(unnamed)";
      const line = css.slice(0, q.index).split("\n").length;
      const own = q[1] ? owners.get(name) : new Set([...owners.values()].flatMap((s) => [...s]));
      if (!own) {
        findings.push({ file, line, msg: `@container ${name}: nothing declares a container with that name` });
        continue;
      }
      const body = blockBody(css, q.index + q[0].length - 1);
      for (const rule of body.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
        const props = [...rule[2].matchAll(/([a-z-]+)\s*:/g)].map((m) => camel(m[1]));
        for (const selector of rule[1].split(",")) {
          // Only the element the selector lands on. `.owner .child` is the
          // normal, working shape and must pass.
          const subject = selector.trim().split(/[\s>+~]+/).pop() ?? "";
          for (const cls of subject.matchAll(/\.([a-zA-Z][\w-]*)/g)) {
            if (own.has(cls[1])) {
              findings.push({
                file,
                line,
                msg: `@container ${name} styles .${cls[1]}, which IS that container — a query cannot style its own container, so this rule never applies`,
              });
            }
            for (const hit of inlineStylesFor(sources, cls[1])) {
              if (hit.style === OPAQUE) {
                findings.push({
                  file: hit.file,
                  line: hit.line,
                  msg: `.${cls[1]} takes its inline style from a variable or spread, and @container ${name} (${file}:${line}) restyles it — write the object out so this check can read it`,
                });
                continue;
              }
              for (const prop of props) {
                const hitProp = overlapping(prop).find((p) => new RegExp(`\\b${p}\\s*:`).test(hit.style));
                if (hitProp) {
                  findings.push({
                    file: hit.file,
                    line: hit.line,
                    msg: `.${cls[1]} sets ${hitProp} inline, and @container ${name} (${file}:${line}) overrides ${prop} — the inline value wins, so the breakpoint does nothing`,
                  });
                }
              }
            }
          }
        }
      }
    }
  }
  return { findings, queries, containers: owners.size };
}

if (SELF_TEST) {
  const dir = mkdtempSync(join(tmpdir(), "container-selftest-"));
  try {
    mkdirSync(join(dir, "src", "styles"), { recursive: true });
    const path = join(dir, "src", "styles", "x.css");
    writeFileSync(
      path,
      // Comment quotes a bad query, to prove the comment strip works.
      `/* @container ghost-in-comment (max-width: 1px) { .a { top: 0 } } */\n` +
        `.panel { container: stream / inline-size; }\n` +
        `.named { container-name: other; }\n` +
        `@container stream (max-width: 640px) {\n` +
        `  .panel { flex-direction: column; }\n` +
        `  .panel .child, .rail { width: auto; }\n` +
        `}\n` +
        `@container ghost (max-width: 640px) { .rail { width: auto; } }\n` +
        `@container other (max-width: 640px) { .rail { width: auto; } }\n` +
        `@container (max-width: 1px) { .named { top: 0; } }\n` +
        `.anon { container-type: inline-size; }\n` +
        `@container (max-width: 2px) { .anon { top: 0; } }\n` +
        `@container other (max-width: 3px) { .banded { border-right: 0; } .opaque { top: 0; } .spready { top: 0; } .flowy { flex-direction: row; } .tpl { left: 0; } }\n`,
    );
    const inlined = [
      {
        file: "x.tsx",
        text:
          `<div className="rail" style={{ width: 240, onClick: () => a > b }} />\n` +
          `<div className="rail other" style={{ padding: 8 }} />\n` +
          `<div style={{ color: "red" }} className="other" />\n` +
          `<div className="rail-x" style={{ width: 1 }} />\n` +
          `<div className="banded" style={{ border: "none" }} />\n` +
          `<div className="opaque" style={opaqueStyle} />\n` +
          `<div className="spready" style={{ ...base }} />\n` +
          `<div className="flowy" style={{ flexFlow: "row" }} />\n` +
          "<div className={`tpl ${extra}`} style={{ left: 1 }} />\n",
      },
    ];
    const { findings } = check([{ file: "x.css", css: stripComments(readFileSync(path, "utf8")) }], inlined);
    const msgs = findings.map((f) => f.msg);
    const selfStyled = msgs.some((m) => m.includes("styles .panel"));
    const ghost = msgs.some((m) => m.includes("@container ghost:"));
    const commentLeaked = msgs.some((m) => m.includes("ghost-in-comment"));
    const inline = msgs.filter((m) => m.includes("sets width inline")).length;
    const unnamed = msgs.some((m) => m.includes("@container (unnamed) styles .named"));
    const anon = msgs.some((m) => m.includes("@container (unnamed) styles .anon"));
    const shorthand = msgs.some((m) => m.includes(".banded sets border inline"));
    const opaque = msgs.some((m) => m.includes(".opaque takes its inline style from a variable"));
    const spread = msgs.some((m) => m.includes(".spready takes its inline style from a variable"));
    const flow = msgs.some((m) => m.includes(".flowy sets flexFlow inline"));
    const template = msgs.some((m) => m.includes(".tpl sets left inline"));
    // selfStyled, ghost, unnamed, anon, shorthand, opaque, spread, flow,
    // template, and the inline width reported once for each of the two queries that override it on
    // `.rail` (`stream`, `other`). `.rail-x` must not count as `.rail`.
    if (!selfStyled || !ghost || !unnamed || !anon || !shorthand || !opaque || !spread || !flow || !template || inline !== 2 || commentLeaked || findings.length !== 11) {
      console.error(
        `self-test FAILED: selfStyled=${selfStyled} ghost=${ghost} unnamed=${unnamed} anon=${anon} shorthand=${shorthand} opaque=${opaque} spread=${spread} flow=${flow} template=${template} inline=${inline} commentLeaked=${commentLeaked} ` +
          `count=${findings.length}\n${JSON.stringify(findings, null, 2)}`,
      );
      process.exit(1);
    }
    console.log(
      "self-test ok — fires on a rule styling its own container (named, unnamed or\n" +
        "  anonymous), on a query naming no container, and on a queried property set inline\n" +
        "  directly, by shorthand or through a variable; ignores a commented query, a\n" +
        "  descendant of the container, an unrelated inline style and a similar class name",
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  process.exit(0);
}

const root = join(process.cwd(), "src");
const sheets = walk(root).map((f) => ({
  file: f.slice(process.cwd().length + 1),
  css: stripComments(readFileSync(f, "utf8")),
}));
const sources = walk(root, [], ".tsx")
  .filter((f) => !f.includes(".test."))
  .map((f) => ({ file: f.slice(process.cwd().length + 1), text: readFileSync(f, "utf8") }));
if (!sources.length) {
  console.error("container-queries: refusing a vacuous pass — no .tsx sources found");
  process.exit(1);
}
const { findings, queries, containers } = check(sheets, sources);

// Green is not evidence anything happened: a scan that found no query
// would report no findings.
if (queries < 2 || containers < 2) {
  console.error(`container-queries: refusing a vacuous pass — ${queries} queries, ${containers} containers`);
  process.exit(1);
}

if (findings.length) {
  console.error(`container-queries: ${findings.length} finding(s)\n`);
  for (const f of findings) console.error(`  ${f.file}:${f.line}  ${f.msg}`);
  process.exit(1);
}

console.log(`container-queries OK — ${queries} queries over ${containers} containers, all able to apply`);
