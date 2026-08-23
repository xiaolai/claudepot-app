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

/**
 * A stub host for the authenticated pass.
 *
 * Only the routes a thread's first paint touches, answered with the
 * smallest shape each consumer reads. Anything else 404s — a screen
 * that needs a route not listed here is a screen this check does not
 * cover, and it should say so by failing rather than by hanging.
 */
const SESSION_ID = 'render-check-session';
function stubHost() {
  const body = (o) => Promise.resolve({
    ok: true, status: 200, json: () => Promise.resolve(o), text: () => Promise.resolve(JSON.stringify(o)),
  });
  return (input) => {
    const url = String(typeof input === 'string' ? input : input?.url || '');
    if (url.includes('/api/me')) return body({ device: 'render-check', passkeys: 0, server_version: '0.0.0' });
    if (url.includes('/transcript')) {
      return body({
        total: 2,
        next_cursor: 2,
        events: [
          { index: 0, kind: 'user', ts: '2026-08-24T00:00:00Z', text: 'hello' },
          { index: 1, kind: 'assistant', ts: '2026-08-24T00:00:01Z', text: 'hi **there**' },
        ],
      });
    }
    if (url.includes('/api/sessions')) {
      return body({
        server_version: '0.0.0',
        sessions: [{
          session_id: SESSION_ID, live: true, addressable: true, status: 'working',
          title: 'a session to open', project_path: '/tmp/p', branch: 'main',
          messages: 2, models: ['claude-opus-5'], tokens: { input: 1, output: 1 },
          last_ts: '2026-08-24T00:00:01Z', has_error: false, unread: 0,
        }],
      });
    }
    if (url.includes('/api/quick-prompts')) return body({ prompts: [{ id: 'q1', name: 'Go', text: 'go on' }] });
    if (url.includes('/api/approvals')) return body({ approvals: [] });
    return Promise.resolve({ ok: false, status: 404, json: () => Promise.resolve({}), text: () => Promise.resolve('') });
  };
}

/** Global restores, run once on the way out — see the note in `render`. */
const pendingRestores = [];
function restoreAll() {
  while (pendingRestores.length) pendingRestores.pop()();
}

async function render(dir, scenario = {}) {
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
  if (scenario.token) {
    dom.window.localStorage.setItem('claudepot.token', scenario.token);
    const f = stubHost();
    dom.window.fetch = f;
    globalThis.fetch = f;
  }

  try {
    // A cache-busting query so repeated runs in one process re-evaluate.
    const url = `${pathToFileURL(join(dir, 'panel.js')).href}?t=${Date.now()}`;
    await import(url);
  } catch (e) {
    errors.push(`import: ${e.message}`);
  }

  await new Promise((r) => setTimeout(r, 500));
  if (scenario.then) {
    try {
      await scenario.then(dom.window);
    } catch (e) {
      errors.push(`scenario: ${e.message}`);
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  const root = dom.window.document.getElementById('root');
  const out = {
    hasComposer: Boolean(dom.window.document.querySelector('textarea')),
    hasSheet: Boolean(dom.window.document.querySelector('[role="dialog"]')),
    children: root?.children.length ?? 0,
    text: (root?.textContent || '').replace(/\s+/g, ' ').trim(),
    errors,
  };
  // Teardown is DEFERRED to process exit, not done here.
  //
  // The panel polls on a `setInterval`, and jsdom's timers are Node
  // timers that outlive `window.close()`. Restoring the globals between
  // renders let one of those fire into a world with no `document`,
  // which killed the process *after* the verdict had been printed — a
  // gate that reports success and then exits non-zero is not a gate.
  // Nothing here needs the globals put back mid-run: the next render
  // installs its own over the top, and the process is short-lived.
  dom.window.close();
  pendingRestores.push(restore);
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

/**
 * The authenticated pass: sign in, open a thread.
 *
 * The sign-in assertion above proves the bundle mounts. It does not
 * reach `Thread`, `Sessions` or `Markdown` — which is how two missing
 * imports (`useEffect`, then `api`) shipped in one commit and turned
 * every thread into a blank screen. Vite does not resolve free
 * identifiers, so nothing before this caught them.
 */
async function openAThread(window) {
  const doc = window.document;
  for (let i = 0; i < 40; i += 1) {
    // The DEEPEST element carrying the title, not the first: every
    // ancestor up to <body> also "contains" the text, and clicking one
    // of those misses the handler. `Item` renders a div rather than a
    // button, so this cannot just query buttons.
    const hits = [...doc.querySelectorAll('div,button')].filter((el) =>
      el.textContent?.includes('a session to open'),
    );
    const card = hits[hits.length - 1];
    if (card) {
      card.click();
      // A live card's own click target may be an ancestor of the text,
      // so walk up a couple of levels too. Clicking a div with no
      // handler is inert, which makes this cheap rather than risky.
      let el = card;
      for (let up = 0; up < 3 && el?.parentElement; up += 1) {
        el = el.parentElement;
        el.click();
      }
      return;
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error('the session card never appeared');
}

/**
 * Open a thread, then open the quick-prompt sheet from `…`.
 *
 * The two pickers share `PickerSheet`, so exercising one proves the
 * sheet chrome mounts; this one is chosen because it is also the path
 * that renders `QuickPicker`'s rows.
 */
async function openTheQuickSheet(window) {
  await openAThread(window);
  const doc = window.document;
  for (let i = 0; i < 40; i += 1) {
    const btn = doc.querySelector('button[aria-label="Send a quick prompt"]');
    if (btn) {
      btn.click();
      return;
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error('the quick-prompt button never appeared');
}

function evaluateSheet(result) {
  const problems = [];
  if (result.errors.length) problems.push(...result.errors.map((e) => e.slice(0, 400)));
  if (!result.hasSheet) problems.push('the quick-prompt sheet did not open');
  // The stub host serves one prompt named `Go`; if the sheet mounted
  // but rendered no rows, the row renderer is what broke.
  if (!/Go/.test(result.text)) {
    problems.push(`the sheet rendered no rows (text was: ${result.text.slice(0, 160)})`);
  }
  return problems;
}

function evaluateThread(result) {
  const problems = [];
  if (result.children === 0) problems.push('#root has no children — React never mounted');
  if (result.errors.length) problems.push(...result.errors.map((e) => e.slice(0, 400)));
  // The composer is the thread's own furniture: if it is there, the
  // whole view rendered rather than throwing on the way.
  if (!result.hasComposer) {
    problems.push(`the thread did not render (text was: ${result.text.slice(0, 200)})`);
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
const threadProblems = evaluateThread(
  await render(outDir, { token: 'render-check-token', then: openAThread }),
);
const sheetProblems = evaluateSheet(
  await render(outDir, { token: 'render-check-token', then: openTheQuickSheet }),
);
clearTimeout(deadline);
if (problems.length || threadProblems.length || sheetProblems.length) {
  console.error('panel render check FAILED');
  for (const p of problems) console.error(`  - signed out: ${p}`);
  for (const p of threadProblems) console.error(`  - thread: ${p}`);
  for (const p of sheetProblems) console.error(`  - sheet: ${p}`);
  restoreAll();
  process.exit(1);
}
console.log('panel render check ok — sign-in, a thread and a picker sheet all render, no console errors');
restoreAll();
// Explicit, so a poll timer the panel scheduled cannot outlive the
// verdict and turn a pass into a non-zero exit.
process.exit(0);
