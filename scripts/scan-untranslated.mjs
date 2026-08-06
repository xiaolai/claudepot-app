#!/usr/bin/env node
// Advisory scan for user-visible English that never made it into a
// catalog.
//
// Deliberately NOT a hard gate. The signal is a heuristic — a bare
// literal in a `title=` could equally be a path, a wire value, or a
// model id, all of which must stay English — so a blocking version
// would either be noisy enough to be ignored or narrow enough to be
// useless. It exists to be run after a UI change and read by a human,
// which is the same role `git diff` plays.
//
// Exit code is always 0. Findings go to stdout.
//
// Usage:
//   node scripts/scan-untranslated.mjs            # whole src tree
//   node scripts/scan-untranslated.mjs --changed  # only files git says changed

import { readFileSync, readdirSync, statSync } from "node:fs";
import { execSync } from "node:child_process";
import { join, dirname, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const SRC = join(ROOT, "src");

/** Files whose English is intentional and documented. */
const EXEMPT_FILES = [
  /\.test\.[tj]sx?$/,
  /\/locales\//,
  /\/lib\/i18n\.ts$/,
  /\/lib\/i18n-error\.ts$/,
  // Log tags, not UI copy.
  /\/ErrorBoundary\.tsx$/,
];

/**
 * Values that look like prose but are data. Each of these was a real
 * false positive during the extraction phases.
 */
const LOOKS_LIKE_DATA =
  /^(?:[A-Z]{2,}|[\w.-]+\.(?:json|md|jsonl|ts|tsx|rs|db)|~?\/|\.\/|https?:|[A-Z]:\\|\$|--?[a-z]|[\w-]+\/[\w-]+|claude[\w-]*|sk-ant|\d)/;

/** Single words are almost always identifiers, units, or wire values. */
const isProse = (s) => {
  const t = s.trim();
  if (t.length < 4 || t.length > 200) return false;
  if (LOOKS_LIKE_DATA.test(t)) return false;
  // At least two words, and at least one lowercase run — "OPUS SON"
  // and "MAX_RETRIES" are not prose.
  return /\s/.test(t) && /[a-z]{2,}/.test(t);
};

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (/\.tsx?$/.test(p)) out.push(p);
  }
  return out;
}

function targetFiles() {
  if (!process.argv.includes("--changed")) return walk(SRC);
  const out = execSync("git diff --name-only HEAD", { cwd: ROOT })
    .toString()
    .split("\n")
    .filter((f) => /^src\/.*\.tsx?$/.test(f))
    .map((f) => join(ROOT, f));
  return out.filter((f) => {
    try {
      return statSync(f).isFile();
    } catch {
      return false;
    }
  });
}

const findings = [];

for (const file of targetFiles()) {
  const rel = relative(ROOT, file);
  if (EXEMPT_FILES.some((re) => re.test(rel))) continue;
  const src = readFileSync(file, "utf8");
  const lines = src.split("\n");

  lines.forEach((line, i) => {
    const n = i + 1;
    // A line already resolving a translation is fine.
    if (/\bt\(|i18n\.t\(|<Trans\b|getFixedT/.test(line)) return;
    // Comments are not UI.
    const code = line.replace(/\/\/.*$/, "");
    if (!code.trim() || /^\s*\*/.test(code)) return;

    // 1. User-visible string props.
    for (const m of code.matchAll(
      /\b(title|aria-label|placeholder|alt|label|confirmLabel|cancelLabel)\s*=\s*"([^"]{4,})"/g,
    )) {
      if (isProse(m[2])) {
        findings.push({ rel, n, kind: m[1], text: m[2] });
      }
    }

    // 2. JSX text nodes: `>Some words<` on one line.
    for (const m of code.matchAll(/>\s*([A-Z][^<>{}"'`]{3,})\s*</g)) {
      if (isProse(m[1])) {
        findings.push({ rel, n, kind: "jsx-text", text: m[1].trim() });
      }
    }
  });
}

const byFile = new Map();
for (const f of findings) {
  if (!byFile.has(f.rel)) byFile.set(f.rel, []);
  byFile.get(f.rel).push(f);
}

if (findings.length === 0) {
  console.log("no untranslated user-visible strings found (advisory scan)");
} else {
  console.log(
    `advisory: ${findings.length} suspect literal(s) in ${byFile.size} file(s).`,
  );
  console.log("Each may be legitimate — paths, wire values, and model ids");
  console.log("stay English on purpose. Read, don't bulk-fix.\n");
  for (const [rel, items] of [...byFile].sort()) {
    console.log(`  ${rel}`);
    for (const it of items.slice(0, 8)) {
      console.log(`    ${it.n}: [${it.kind}] ${it.text}`);
    }
    if (items.length > 8) console.log(`    … ${items.length - 8} more`);
  }
}

process.exit(0);
