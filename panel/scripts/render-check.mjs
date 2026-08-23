// Does the built bundle actually render?
//
// "It builds" and "it renders" are different claims, and only the first
// is checked by `vite build`. P1's acceptance was "the served page
// renders the design system with zero console errors" — a claim nobody
// could make without opening a browser, which is how a bundle that
// throws on its first render ships looking green.
//
// jsdom is not a browser: it has no layout, so this cannot say the
// design *looks* right. It answers the narrower question that actually
// regresses — did the module graph evaluate, did React mount, did
// anything throw — and it answers it in CI, on a machine with no screen.
//
// Run against the committed output, not the sources: the committed
// output is the artifact, and a source change nobody rebuilt is exactly
// the failure worth catching.
//
// ## Why the bundle is imported and not eval'd
//
// It used to be `window.eval(source)`. That stopped working the moment
// mermaid was added: code-splitting turned the entry into a real ES
// module, `export` is a syntax error in a classic script, and the check
// went red for a bundle that was perfectly fine. Loading it through
// node's own ESM loader with browser globals installed runs the artifact
// the way a browser would — and keeps working when the chunk graph
// changes again.
//
//   node scripts/render-check.mjs [outDir]
//   node scripts/render-check.mjs --self-test
//
// The self-test builds a deliberately broken bundle in a temp directory
// and runs the same code path over it. A guard nobody has watched go red
// is indistinguishable from one that cannot.
import { JSDOM, VirtualConsole } from 'jsdom';
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { dirname, join, resolve } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const DEFAULT_OUT = resolve(HERE, '../../crates/claudepot-core/src/remote/assets/panel');

const args = process.argv.slice(2);
const selfTest = args.includes('--self-test');
const outDir = args.find((a) => !a.startsWith('-')) || DEFAULT_OUT;

/** Install a jsdom window's globals so an ES module sees a browser. */
function installGlobals(window) {
  const restore = [];
  const set = (key, value) => {
    const had = Object.prototype.hasOwnProperty.call(globalThis, key);
    const prev = globalThis[key];
    Object.defineProperty(globalThis, key, { value, configurable: true, writable: true });
    restore.push(() => {
      if (had) Object.defineProperty(globalThis, key, { value: prev, configurable: true, writable: true });
      else delete globalThis[key];
    });
  };

  // The three jsdom lacks that any React app touches. Stubbing them is
  // not papering over a bug — a real browser has all three.
  window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
  });
  if (!window.ResizeObserver) {
    window.ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    };
  }
  Object.defineProperty(window, 'crypto', { value: globalThis.crypto, configurable: true });

  // No network. Every request the panel makes must be tolerated as a
  // failure — that is the offline path, and the one a phone hits first.
  // A bundle that only renders when the host answers is a bundle that
  // renders a blank screen on a dropped connection.
  window.fetch = () => Promise.reject(new TypeError('offline (render-check)'));

  for (const key of [
    'window',
    'document',
    'navigator',
    'location',
    'localStorage',
    'sessionStorage',
    'matchMedia',
    'ResizeObserver',
    'fetch',
    'HTMLElement',
    'Element',
    'Node',
    'Event',
    'CustomEvent',
    'MutationObserver',
    'getComputedStyle',
    'requestAnimationFrame',
    'cancelAnimationFrame',
    'DOMParser',
  ]) {
    if (key in window) set(key, window[key]);
  }

  // `window` must BE the global object, not merely live on it.
  //
  // The vendored design system publishes itself with
  // `Object.assign(window, { Ico, Btn, … })` and then reads those back as
  // bare identifiers, which works in a browser because `window ===
  // globalThis`. Mirroring one way is not enough: `globals.js` sets
  // `window.React` and `ds-kit.jsx` reads bare `React`. Pointing `window`
  // at the global makes both directions the same object, which is the
  // arrangement the bundle was written against.
  set('window', globalThis);
  return () => restore.forEach((f) => f());
}

async function render(dir) {
  const errors = [];
  const vc = new VirtualConsole();
  vc.on('jsdomError', (e) => errors.push(`jsdomError: ${e.message}`));
  vc.on('error', (...a) => errors.push(`console.error: ${a.join(' ')}`));

  const dom = new JSDOM(readFileSync(join(dir, 'index.html'), 'utf8'), {
    // A named origin, not an IP: the panel reads `location.hostname` to
    // decide whether this origin can host a passkey at all.
    url: 'https://panel-render-check.local/',
    pretendToBeVisual: true,
    virtualConsole: vc,
  });
  const restore = installGlobals(dom.window);

  try {
    // A cache-busting query so repeated runs in one process re-evaluate.
    const url = `${pathToFileURL(join(dir, 'panel.js')).href}?t=${Date.now()}`;
    await import(url);
  } catch (e) {
    errors.push(`import: ${e.message}`);
  }

  await new Promise((r) => setTimeout(r, 500));
  const root = dom.window.document.getElementById('root');
  const out = {
    children: root?.children.length ?? 0,
    text: (root?.textContent || '').replace(/\s+/g, ' ').trim(),
    errors,
  };
  restore();
  dom.window.close();
  return out;
}

function evaluate(result) {
  const problems = [];
  if (result.children === 0) problems.push('#root has no children — React never mounted');
  if (result.errors.length) problems.push(...result.errors.map((e) => e.slice(0, 400)));
  // The sign-in screen is what an unauthenticated load must reach. A
  // mounted-but-empty tree would pass a children check and show nothing.
  if (!/Sign in/i.test(result.text)) {
    problems.push(`the sign-in screen did not render (text was: ${result.text.slice(0, 160)})`);
  }
  return problems;
}

if (selfTest) {
  // A bundle that mounts nothing, in a throwaway directory, through the
  // same code path.
  const dir = mkdtempSync(join(tmpdir(), 'panel-render-selftest-'));
  try {
    writeFileSync(join(dir, 'index.html'), '<!doctype html><html><body><div id="root"></div></body></html>');
    writeFileSync(join(dir, 'panel.js'), 'export const nothingHappens = true;\n');
    const problems = evaluate(await render(dir));
    if (problems.length === 0) {
      console.error('self-test FAILED: a bundle that mounts nothing reported no problems');
      process.exit(1);
    }
    console.log(`self-test ok — the guard fires (${problems.length} problem(s) reported)`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  process.exit(0);
}

/**
 * A hard ceiling on the whole check.
 *
 * jsdom has no layout engine, so anything that measures — mermaid asking
 * for `getBBox`, most obviously — can spin rather than fail. This check
 * does not render a diagram today, but it is a CI gate, and a gate that
 * can hang is worse than one that can fail: the run sits there until the
 * job's own timeout and reports nothing useful.
 */
const DEADLINE_MS = 60_000;
const deadline = setTimeout(() => {
  console.error(`panel render check FAILED — no verdict within ${DEADLINE_MS / 1000}s`);
  process.exit(1);
}, DEADLINE_MS);
deadline.unref?.();

const problems = evaluate(await render(outDir));
clearTimeout(deadline);
if (problems.length) {
  console.error('panel render check FAILED');
  for (const p of problems) console.error(`  - ${p}`);
  process.exit(1);
}
console.log('panel render check ok — mounted, sign-in screen present, no console errors');
