#!/usr/bin/env node
// Locale-catalog completeness gate.
//
// English is the source of truth: every en key must exist in every
// other locale, with the same interpolation placeholders and the same
// <Trans> tag set. A missing zh key renders English at runtime
// (fallbackLng), which is a silent half-translated UI rather than a
// visible failure — so it has to fail here instead.
//
// Node, not vitest, so CI can gate the catalogs without booting jsdom
// and the whole React module graph. Same reasoning as
// `scripts/capture-screenshots.mjs`: no new dependency.
//
// Usage: node scripts/check-catalogs.mjs [--quiet]

import { readdirSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

// Overridable so the gate itself can be exercised against fixtures —
// a check nobody has watched fail is indistinguishable from one that
// cannot fail (see `check:envvar-layout`, defined and never invoked).
const LOCALES_DIR =
  process.env.CLAUDEPOT_LOCALES_DIR ??
  join(dirname(fileURLToPath(import.meta.url)), "..", "src", "locales");
const SOURCE_LOCALE = "en";

/** Plural suffixes i18next resolves via Intl.PluralRules. A key ending
 *  in one of these is a *variant* of a base key, not a key of its own —
 *  parity is asserted on the bases, because zh-CN legitimately carries
 *  only `_other` while en carries `_one` + `_other`. */
const PLURAL_SUFFIXES = [
  "_zero",
  "_one",
  "_two",
  "_few",
  "_many",
  "_other",
];

function flatten(obj, prefix = "", out = new Map()) {
  for (const [k, v] of Object.entries(obj)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === "object" && !Array.isArray(v)) flatten(v, key, out);
    else out.set(key, v);
  }
  return out;
}

const pluralBase = (key) => {
  for (const s of PLURAL_SUFFIXES) {
    if (key.endsWith(s)) return key.slice(0, -s.length);
  }
  return key;
};

/** `{{name}}` and `{{name, formatter}}` — the name is what must match. */
function placeholders(value) {
  if (typeof value !== "string") return new Set();
  return new Set(
    [...value.matchAll(/\{\{\s*([\w.-]+)\s*[,}]/g)].map((m) => m[1]),
  );
}

/**
 * `<0>`, `<strong>`, `<code/>` — a translation that drops, renames,
 * unbalances, or duplicates a tag the code supplies renders a mangled
 * sentence, so tags are part of the contract, not prose.
 *
 * A **multiset**, not a name set and not an ordered sequence:
 *
 * - A name set loses open-vs-close and multiplicity, so `<b>x</b>` →
 *   `<b>x` and a doubled tag both compared equal and passed.
 * - An ordered sequence is too strict. `<Trans>` maps *named*
 *   components, so a translation is free to reorder them — Chinese
 *   syntax routinely does, and two real `config` strings do exactly
 *   that. Requiring identical order would reject correct translations.
 */
function transTagSignature(value) {
  if (typeof value !== "string") return "";
  const counts = new Map();
  for (const m of value.matchAll(/<(\/?)([\w-]+)\s*(\/?)>/g)) {
    const token = m[3] ? `${m[2]}/` : `${m[1]}${m[2]}`;
    counts.set(token, (counts.get(token) ?? 0) + 1);
  }
  return [...counts]
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([t, n]) => `${t}×${n}`)
    .join(",");
}

// Deliberately NOT checked: whether a string's tags are balanced within
// itself. A catalog value carries no record of how it is rendered, and
// prose legitimately contains angle brackets — `config.lifecycle.
// suffixBody` says `Disable as "<name>-N" instead?` and is passed to a
// plain `t()`, never to `<Trans>`. Flagging that is a false positive
// with no offsetting coverage: a translation that drops a closing tag
// the source had already fails the cross-locale multiset comparison
// above, which is the check that can actually tell markup from prose
// (because the source is the reference).

const read = (locale, file) =>
  JSON.parse(readFileSync(join(LOCALES_DIR, locale, file), "utf8"));

const locales = readdirSync(LOCALES_DIR, { withFileTypes: true })
  .filter((d) => d.isDirectory())
  .map((d) => d.name);

if (!locales.includes(SOURCE_LOCALE)) {
  console.error(`FAIL: no ${SOURCE_LOCALE}/ directory under src/locales`);
  process.exit(1);
}

const sourceFiles = readdirSync(join(LOCALES_DIR, SOURCE_LOCALE))
  .filter((f) => f.endsWith(".json"))
  .sort();

const errors = [];
const stats = [];

// A gate that checks nothing must not report OK. With only `en/` present
// the loop below never runs and every parity assertion is vacuous — the
// script would print "catalogs OK" having compared zero pairs, which is
// indistinguishable from a passing check right up until someone deletes
// a locale directory.
const targetLocales = locales.filter((l) => l !== SOURCE_LOCALE);
if (targetLocales.length === 0) {
  console.error(
    `FAIL: ${SOURCE_LOCALE}/ is the only locale under ${LOCALES_DIR} — ` +
      "there is nothing to check parity against",
  );
  process.exit(1);
}

for (const target of targetLocales) {
  const targetFiles = new Set(
    readdirSync(join(LOCALES_DIR, target)).filter((f) => f.endsWith(".json")),
  );

  for (const file of sourceFiles) {
    const ns = file.replace(/\.json$/, "");
    if (!targetFiles.has(file)) {
      errors.push(`${target}/${file}: namespace file missing`);
      continue;
    }

    let src, dst;
    try {
      src = flatten(read(SOURCE_LOCALE, file));
    } catch (e) {
      errors.push(`${SOURCE_LOCALE}/${file}: invalid JSON — ${e.message}`);
      continue;
    }
    try {
      dst = flatten(read(target, file));
    } catch (e) {
      errors.push(`${target}/${file}: invalid JSON — ${e.message}`);
      continue;
    }

    const srcBases = new Set([...src.keys()].map(pluralBase));
    const dstBases = new Set([...dst.keys()].map(pluralBase));

    for (const base of srcBases) {
      if (!dstBases.has(base)) errors.push(`${target}/${ns}: missing "${base}"`);
    }
    for (const base of dstBases) {
      if (!srcBases.has(base)) {
        errors.push(`${target}/${ns}: orphaned "${base}" (not in ${SOURCE_LOCALE})`);
      }
    }

    // Placeholder + tag parity, compared per plural base so an en
    // `_one`/`_other` pair is checked against zh's single `_other`.
    const byBase = (map) => {
      const m = new Map();
      for (const [k, v] of map) {
        const b = pluralBase(k);
        const ph = m.get(b) ?? { ph: new Set(), tags: new Set() };
        for (const p of placeholders(v)) ph.ph.add(p);
        // Plural variants of one base must agree on tag structure, so
        // collecting signatures into a set is right: a base whose
        // `_one` and `_other` differ structurally is itself a bug, and
        // shows up as a set-size mismatch below.
        ph.tags.add(transTagSignature(v));
        m.set(b, ph);
      }
      return m;
    };
    const srcMeta = byBase(src);
    const dstMeta = byBase(dst);

    for (const [base, s] of srcMeta) {
      const d = dstMeta.get(base);
      if (!d) continue; // already reported as missing
      for (const p of s.ph) {
        if (!d.ph.has(p)) {
          errors.push(`${target}/${ns}: "${base}" drops placeholder {{${p}}}`);
        }
      }
      for (const p of d.ph) {
        if (!s.ph.has(p)) {
          errors.push(
            `${target}/${ns}: "${base}" adds unknown placeholder {{${p}}}`,
          );
        }
      }
      // Compare tag multisets both ways. One-directional checking let a
      // translation add a tag the source never had; comparing name sets
      // let it unbalance or duplicate one.
      const sTags = [...s.tags].sort().join(" | ");
      const dTags = [...d.tags].sort().join(" | ");
      if (sTags !== dTags) {
        errors.push(
          `${target}/${ns}: "${base}" tag structure differs — ` +
            `${SOURCE_LOCALE}(${sTags || "none"}) vs ${target}(${dTags || "none"})`,
        );
      }
    }

    // An empty string renders as an empty UI element — almost always a
    // placeholder left behind rather than an intentional blank label.
    // Checked on BOTH sides: English is the runtime source *and* the
    // fallback every other locale falls back to, so an empty en value
    // blanks the surface in every language at once.
    for (const [label, map] of [
      [target, dst],
      [SOURCE_LOCALE, src],
    ]) {
      for (const [k, v] of map) {
        if (typeof v === "string" && v.trim() === "") {
          errors.push(`${label}/${ns}: "${k}" is empty`);
        }
      }
    }

    stats.push({ target, ns, keys: srcBases.size });
  }
}

// Namespace drift: the gate discovers catalogs from disk, but i18next
// loads a hand-written list. Without this, adding `locales/*/foo.json`
// passes every check above and is simply never loaded — the strings
// render as raw keys with nothing red. Parsing the `ns:` array is
// enough to catch that; it does not need to understand the module.
// Skipped when CLAUDEPOT_LOCALES_DIR points at a fixture — a fixture
// has catalogs but no `i18n.ts` beside it.
if (!process.env.CLAUDEPOT_LOCALES_DIR) {
  const i18nSrc = readFileSync(
    join(LOCALES_DIR, "..", "lib", "i18n.ts"),
    "utf8",
  );
  // `resources` is checked alongside `ns`, because they are two separate
  // hand-written lists. A namespace in `ns` but absent from `resources`
  // leaves i18next with the namespace declared and no catalog bundled —
  // it renders raw keys, and checking only `ns` waved that through.
  //
  // Parsed PER LOCALE, not as one flat set: a first attempt collected
  // every `foo: enFoo,`-shaped line anywhere in the file, so deleting a
  // namespace from the `en` block still passed on the strength of the
  // `zh-CN` block's copy of the same name.
  const resourcesByLocale = new Map();
  const resourcesBlock = i18nSrc.match(/\bresources:\s*\{([\s\S]*?)\n {2}\}/);
  if (resourcesBlock) {
    for (const m of resourcesBlock[1].matchAll(
      /["']?([\w-]+)["']?:\s*\{([^}]*)\}/g,
    )) {
      resourcesByLocale.set(
        m[1],
        new Set([...m[2].matchAll(/([\w-]+):\s*\w+,/g)].map((x) => x[1])),
      );
    }
  }
  const resourceNames = new Set(
    [...resourcesByLocale.values()].flatMap((s) => [...s]),
  );
  const nsBlock = i18nSrc.match(/\bns:\s*\[([\s\S]*?)\]/);
  if (!nsBlock) {
    errors.push(
      "src/lib/i18n.ts: could not find the `ns: [...]` array — the " +
        "namespace-drift check cannot run, so it must not silently pass",
    );
  } else {
    const registered = new Set(
      [...nsBlock[1].matchAll(/["']([\w-]+)["']/g)].map((m) => m[1]),
    );
    const onDisk = new Set(sourceFiles.map((f) => f.replace(/\.json$/, "")));
    if (resourceNames.size === 0) {
      errors.push(
        "src/lib/i18n.ts: could not find any `resources` entries — the " +
          "bundled-catalog check cannot run, so it must not silently pass",
      );
    }
    // Every locale on disk must HAVE a resources block. Iterating only
    // the blocks that were found means deleting `resources["zh-CN"]`
    // wholesale reports nothing — the per-namespace loop below simply
    // has one fewer locale to check, which is a false negative shaped
    // exactly like success.
    for (const loc of locales) {
      if (!resourcesByLocale.has(loc)) {
        errors.push(
          `src/lib/i18n.ts: locale "${loc}" has catalogs on disk but no ` +
            `\`resources.${loc}\` block — i18next bundles nothing for it`,
        );
      }
    }
    for (const ns of onDisk) {
      if (!registered.has(ns)) {
        errors.push(
          `src/lib/i18n.ts: catalog "${ns}" exists on disk but is not in ` +
            `the \`ns\` array — i18next will never load it`,
        );
      }
      for (const [loc, names] of resourcesByLocale) {
        if (!names.has(ns)) {
          errors.push(
            `src/lib/i18n.ts: namespace "${ns}" has no entry under ` +
              `\`resources.${loc}\` — i18next loads no ${loc} catalog for it`,
          );
        }
      }
    }
    for (const ns of registered) {
      if (!onDisk.has(ns)) {
        errors.push(
          `src/lib/i18n.ts: namespace "${ns}" is registered but has no ` +
            `${SOURCE_LOCALE}/${ns}.json`,
        );
      }
    }
  }
}

if (!process.argv.includes("--quiet")) {
  const total = stats.reduce((n, s) => n + s.keys, 0);
  const namespaces = new Set(stats.map((s) => s.ns)).size;
  console.log(
    `catalogs: ${namespaces} namespaces, ${total} keys checked across ${locales.length} locales`,
  );
}

if (errors.length) {
  console.error(`\nFAIL: ${errors.length} catalog problem(s)\n`);
  for (const e of errors.slice(0, 100)) console.error(`  ${e}`);
  if (errors.length > 100) {
    console.error(`  … and ${errors.length - 100} more`);
  }
  process.exit(1);
}

console.log("catalogs OK");
