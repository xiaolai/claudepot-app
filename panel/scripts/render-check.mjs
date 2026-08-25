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

/**
 * Install a jsdom window's globals so an ES module sees a browser.
 *
 * `width`, when given, makes `ResizeObserver` report that width for
 * anything observed. The panel's whole responsive system hangs off one
 * observation of its own shell — see `Panel.jsx` — so a no-op observer
 * pins every pass to the phone step and the wide layout is code no
 * check can reach.
 */
function installGlobals(window, width) {
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
  window.ResizeObserver = class {
    constructor(cb) {
      this.cb = cb;
    }
    observe(target) {
      // jsdom has no layout, so a real measurement is always zero. A
      // declared width is the only way to say "this panel is a tablet".
      if (width) this.cb([{ target, contentRect: { width, height: 900 } }], this);
    }
    unobserve() {}
    disconnect() {}
  };
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

  // `history` and the window-level listener pair.
  //
  // These cannot ride the copy loop above. `window` is about to become
  // `globalThis`, so a bare copy of `addEventListener` would be invoked
  // with the wrong receiver and jsdom would refuse it; and Node's
  // `globalThis` has neither `history` nor `addEventListener` of its
  // own, so without this every `window.addEventListener` in the bundle
  // is a TypeError.
  //
  // That is not a hypothetical either. The panel's history integration
  // (open a thread, push an entry; OS back gesture pops it) threw
  // inside its mount effect here, React unmounted the whole tree, and
  // all seven scenarios went blank — including sign-in, which never
  // opens a thread. The feature now guards itself and fails off, so
  // WITHOUT these three lines this check would pass while exercising
  // nothing: green because the feature was disabled, which is the one
  // kind of green worth distrusting.
  set('history', window.history);
  set('addEventListener', window.addEventListener.bind(window));
  set('removeEventListener', window.removeEventListener.bind(window));

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
function stubHost(log = []) {
  const body = (o) => Promise.resolve({
    ok: true, status: 200, json: () => Promise.resolve(o), text: () => Promise.resolve(JSON.stringify(o)),
  });
  return (input, init) => {
    const url = String(typeof input === 'string' ? input : input?.url || '');
    // Recorded so the outbox scenario can assert the drain actually
    // SENT, rather than merely having dropped the entry.
    if (url.endsWith('/prompt')) {
      log.push({ url, key: init?.headers?.['Idempotency-Key'] ?? null });
      return body({ ok: true });
    }
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
    // The command picker's two endpoints. Its fetch, row renderer,
    // argument step and staging are a different code path from the
    // quick picker's, sharing only the sheet chrome.
    if (/\/commands\/[^/]+$/.test(url)) {
      return body({ name: 'audit-fix', text: 'Audit and fix everything.', restricts_tools: true });
    }
    if (url.includes('/commands')) {
      return body({
        commands: [
          // `argument_hint` is what makes the picker two-step, and the
          // two-step path — args field, then stage — is the half a
          // single-click scenario never reaches.
          { name: 'audit-fix', description: 'Audit then fix', argument_hint: '<scope>', source: 'project', body_chars: 14208, restricts_tools: true, pins_model: false },
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
  const restore = installGlobals(dom.window, scenario.width);
  // A fresh outbox per pass: it is localStorage, and jsdom gives each
  // window its own, but the scenario needs to be able to read it back.
  const prompts = [];
  if (scenario.token) {
    dom.window.localStorage.setItem('claudepot.token', scenario.token);
    const f = stubHost(prompts);
    dom.window.fetch = f;
    globalThis.fetch = f;
    // Handed to the scenario so it can go offline and come back.
    dom.window.__host = f;
    dom.window.__prompts = prompts;
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
    // Opening a thread is a navigation and takes a history entry, so
    // the OS back gesture closes the conversation instead of leaving
    // the app. Recorded here because it is invisible in the rendered
    // text — and because the feature guards itself and fails off, so
    // without this the check would pass while exercising nothing.
    threadEntry: dom.window.history.state?.panelThread ?? null,
    hasComposer: Boolean(dom.window.document.querySelector('textarea')),
    hasSheet: Boolean(dom.window.document.querySelector('[role="dialog"]')),
    // The command picker's ARGUMENT step. The hint is a `placeholder`,
    // not text, so it never reaches `textContent` — an assertion on the
    // rendered string would have looked for something that cannot be
    // there and failed for the wrong reason.
    hasCommandArgs: Boolean(
      dom.window.document.querySelector('input[aria-label^="Arguments for"]'),
    ),
    // The staged-command chip in the composer, which is what `stage()`
    // produces. Identified by the remove button's label so the probe
    // does not depend on the chip's wording.
    hasStagedCommand: Boolean(
      dom.window.document.querySelector('button[aria-label^="Remove "]'),
    ),
    // The measured step, read off the element that carries it. `sm`
    // when nothing declared a width, which is every other pass.
    bp: dom.window.document.querySelector('.panel')?.getAttribute('data-bp') ?? null,
    // The thread's Back chevron. Present when the thread COVERS the
    // list, absent when it sits beside it — which is the one behaviour
    // that distinguishes the two layouts from the outside.
    hasBack: Boolean(dom.window.document.querySelector('button[aria-label="Back"]')),
    children: root?.children.length ?? 0,
    text: (root?.textContent || '').replace(/\s+/g, ' ').trim(),
    // What the outbox scenario needs: whether a held row is on screen,
    // what is actually in storage, and what reached the host.
    hasQueuedRow: Boolean(
      dom.window.document.querySelector('button[aria-label="Discard this queued message"]'),
    ),
    outbox: (() => {
      try {
        return JSON.parse(dom.window.localStorage.getItem('claudepot.outbox') || '{}');
      } catch {
        return null;
      }
    })(),
    prompts,
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

/**
 * Open a thread, then open the SLASH-command sheet from `/`.
 *
 * The quick sheet below proves the shared chrome mounts; it cannot
 * prove this one does. `CommandPicker` has its own fetch, its own row,
 * an argument step the other picker has no equivalent of, and the
 * staging path that puts thousands of words into the composer — none of
 * which the quick-prompt scenario touches. A render check that opens
 * one of two pickers reports on one of two pickers.
 */
async function openTheCommandSheet(window) {
  await openAThread(window);
  const doc = window.document;
  let opened = false;
  for (let i = 0; i < 40 && !opened; i += 1) {
    const btn = doc.querySelector('button[aria-label="Insert a command"]');
    if (btn) {
      btn.click();
      opened = true;
      break;
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  if (!opened) throw new Error('the insert-a-command button never appeared');

  // Then drive the ARGUMENT step and actually stage it. The stub
  // command declares an `argument_hint`, so tapping the row opens the
  // args field rather than staging immediately — and staging is the
  // path that puts thousands of words into the composer, which is the
  // whole reason this picker commits on a second press rather than the
  // first. Stopping at the row would leave that press untested while
  // the comment above claimed otherwise.
  for (let i = 0; i < 40; i += 1) {
    const row = [...doc.querySelectorAll('button, [role="button"]')].find((el) =>
      /audit-fix/.test(el.textContent || ''),
    );
    if (row) {
      row.click();
      break;
    }
    await new Promise((r) => setTimeout(r, 100));
  }

  await new Promise((r) => setTimeout(r, 150));
}

/**
 * …and the second press, which is a different END STATE.
 *
 * `stage()` calls `onClose()`, so once Insert lands the sheet is gone
 * and the chip is in the composer. Asserting "sheet open with an args
 * field" and "sheet closed with a staged chip" in one scenario is
 * asserting two mutually exclusive things; the first version did
 * exactly that and failed on its own success.
 */
async function stageACommand(window) {
  await openTheCommandSheet(window);
  const doc = window.document;
  for (let i = 0; i < 40; i += 1) {
    const insert = [...doc.querySelectorAll('button, [role="button"]')].find(
      (el) => (el.textContent || '').trim() === 'Insert',
    );
    if (insert) {
      insert.click();
      break;
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  await new Promise((r) => setTimeout(r, 250));
}

function evaluateCommandSheet(result) {
  const problems = [];
  if (result.errors.length) problems.push(...result.errors.map((e) => e.slice(0, 400)));
  if (!result.hasSheet) problems.push('the slash-command sheet did not open');
  // The stub host serves one command named `audit-fix`.
  if (!/audit-fix/.test(result.text)) {
    problems.push(`the sheet rendered no command rows (text was: ${result.text.slice(0, 160)})`);
  }
  // The argument step, probed in the DOM: the hint is a `placeholder`
  // and never appears in `textContent`. If this is absent after the row
  // was tapped, only the list rendered and the two-step path — the one
  // that ends in thousands of words landing in the composer — is still
  // unexercised.
  if (!result.hasCommandArgs) {
    problems.push(
      `the argument step did not render (text was: ${result.text.slice(0, 200)})`,
    );
  }
  return problems;
}

/**
 * The end of the command path: Insert pressed, sheet closed, expansion
 * in the composer as a chip.
 */
function evaluateStaging(result) {
  const problems = [];
  if (result.errors.length) problems.push(...result.errors.map((e) => e.slice(0, 400)));
  if (result.hasSheet) {
    problems.push('the sheet stayed open after Insert — stage() should close it');
  }
  if (!result.hasStagedCommand) {
    problems.push(
      `Insert did not stage the command (text was: ${result.text.slice(0, 200)})`,
    );
  }
  return problems;
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

/** React tracks a controlled value, so a raw `.value =` is ignored. */
function typeInto(window, el, value) {
  const proto = Object.getPrototypeOf(el);
  Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, value);
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
}

/**
 * Type while the Mac is unreachable, then let it come back.
 *
 * The one path in the panel that sends a message the user is not
 * present for, and the one nothing else exercises end to end: the store
 * has unit tests, the drain has none. Both halves are asserted — that
 * the composer HELD rather than refused, and that reconnecting actually
 * put the text on the wire.
 */
async function queueWhileOffline(window) {
  await openAThread(window);
  const doc = window.document;

  let ta = null;
  for (let i = 0; i < 40 && !ta; i += 1) {
    ta = doc.querySelector('textarea');
    if (!ta) await new Promise((r) => setTimeout(r, 100));
  }
  if (!ta) throw new Error('the composer never appeared');

  // Cut the wire. `api.js` classifies a TypeError from fetch as
  // `OfflineError`, which is what flips `conn`.
  const dead = () => Promise.reject(new TypeError('offline (render-check)'));
  window.fetch = dead;
  globalThis.fetch = dead;
  // Wait past one poll so `conn` is actually `offline` — sending before
  // that would take the normal path and prove nothing.
  await new Promise((r) => setTimeout(r, 4600));

  typeInto(window, ta, 'thought of it on the train');
  await new Promise((r) => setTimeout(r, 60));
  ta.closest('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await new Promise((r) => setTimeout(r, 400));

  const stored = JSON.parse(window.localStorage.getItem('claudepot.outbox') || '{}');
  const held = Object.values(stored).flat();
  if (held.length !== 1) {
    throw new Error(`offline send did not hold the message (outbox held ${held.length})`);
  }
  if (window.__prompts.length !== 0) {
    throw new Error('the message went to the host while it was unreachable');
  }
  if (!doc.querySelector('button[aria-label="Discard this queued message"]')) {
    throw new Error('the held message is not on screen — nothing says it exists');
  }

  // The Mac comes back.
  window.fetch = window.__host;
  globalThis.fetch = window.__host;
  await new Promise((r) => setTimeout(r, 6000));

  if (window.__prompts.length !== 1) {
    throw new Error(`the drain did not send (${window.__prompts.length} prompts reached the host)`);
  }
  // The idempotency key must be the entry's own id, or a replayed drain
  // is a second message rather than the same one.
  if (window.__prompts[0].key !== held[0].id) {
    throw new Error('the drain minted a new idempotency key instead of replaying the entry id');
  }
}

/**
 * The wide layout: rail on the left, list and thread side by side.
 *
 * Asserted through behaviour rather than through the rail's markup,
 * because "there is a `<nav>`" is true of the phone layout too. What is
 * only true at ≥900px is that opening a thread leaves the list on
 * screen — and that there is therefore nothing to go Back from.
 */
function evaluateWide(result) {
  const problems = [];
  if (result.children === 0) problems.push('#root has no children — React never mounted');
  if (result.errors.length) problems.push(...result.errors.map((e) => e.slice(0, 400)));
  if (result.bp !== 'lg') {
    problems.push(`the panel did not step to the wide layout (data-bp was ${result.bp})`);
  }
  if (!result.hasComposer) {
    problems.push(`the thread did not render in its pane (text was: ${result.text.slice(0, 200)})`);
  }
  // Both halves on screen at once is the entire point of this step.
  if (!/a session to open/.test(result.text)) {
    problems.push('the list was covered by the thread instead of sitting beside it');
  }
  if (result.hasBack) {
    problems.push('the thread kept its Back chevron in a pane it never covered the list from');
  }
  return problems;
}

/** After the round trip: nothing held, nothing on screen, one sent. */
function evaluateOutbox(result) {
  const problems = [];
  // The scenario itself throws on every interesting failure, and those
  // arrive here as `scenario: …` errors.
  if (result.errors.length) problems.push(...result.errors.map((e) => e.slice(0, 400)));
  if (Object.keys(result.outbox || {}).length) {
    problems.push('the outbox still holds a message the host accepted');
  }
  if (result.hasQueuedRow) problems.push('a sent message is still rendered as queued');
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
  // The panel kept no history at all before this — one entry for its
  // whole life — so iOS Safari's edge swipe and Android's back gesture
  // left the app rather than closing the thread.
  if (result.threadEntry == null) {
    problems.push(
      'opening a thread pushed no history entry — the OS back gesture would exit the app',
    );
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
const DEADLINE_MS = 90_000;
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
const cmdProblems = evaluateCommandSheet(
  await render(outDir, { token: 'render-check-token', then: openTheCommandSheet }),
);
const stageProblems = evaluateStaging(
  await render(outDir, { token: 'render-check-token', then: stageACommand }),
);
const outboxProblems = evaluateOutbox(
  await render(outDir, { token: 'render-check-token', then: queueWhileOffline }),
);
const wideProblems = evaluateWide(
  await render(outDir, { token: 'render-check-token', then: openAThread, width: 1200 }),
);
clearTimeout(deadline);
if (
  problems.length ||
  threadProblems.length ||
  sheetProblems.length ||
  cmdProblems.length ||
  stageProblems.length ||
  outboxProblems.length ||
  wideProblems.length
) {
  console.error('panel render check FAILED');
  for (const p of problems) console.error(`  - signed out: ${p}`);
  for (const p of threadProblems) console.error(`  - thread: ${p}`);
  for (const p of sheetProblems) console.error(`  - quick sheet: ${p}`);
  for (const p of cmdProblems) console.error(`  - command sheet: ${p}`);
  for (const p of stageProblems) console.error(`  - staging: ${p}`);
  for (const p of outboxProblems) console.error(`  - offline queue: ${p}`);
  for (const p of wideProblems) console.error(`  - wide layout: ${p}`);
  restoreAll();
  process.exit(1);
}
console.log(
  'panel render check ok — sign-in, a thread, both picker sheets, staging a command, ' +
    'holding a message offline and draining it, and the wide two-pane layout all work, ' +
    'no console errors',
);
restoreAll();
// Explicit, so a poll timer the panel scheduled cannot outlive the
// verdict and turn a pass into a non-zero exit.
process.exit(0);
