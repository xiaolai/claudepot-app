// Does the committed bundle lay out correctly in a real browser?
//
// `render-check.mjs` runs in jsdom, which has no layout, and `widths.test.js`
// reads source. Neither sees the faults this measures: a separator opening a
// wrapped line, a badge over its icon, a placeholder cut in half, a control
// past the edge of a clipping card. Written source-only first, those guards
// let 23 of 26 sideways regressions through: a badge pulled back with a
// margin reads as correct to any scan of `left: '100%'`.
//
// Drives Chrome over its own debugging protocol, with no dependency, against
// the committed output and a stub host. The stub deliberately carries the
// hard rows (a long branch, an empty project path); the check fails if those
// rows were not on screen, so a blank panel cannot pass.
//
//   node scripts/layout-check.mjs [outDir]
//   node scripts/layout-check.mjs --self-test
//
// The self-test injects each fault and requires its named failure. A guard
// nobody has watched go red is indistinguishable from one that cannot.
//
// Exit: 0 pass · 1 a check failed, or Chrome failed · 2 no Chrome here
// (CHROME_PATH sets one). Under CI a missing Chrome is 1, because a skipped
// gate is no gate.
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, extname, join, normalize, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const SELF_TEST = args.includes('--self-test');
const OUT = resolve(args.find((a) => !a.startsWith('--')) ?? join(HERE, '../../crates/claudepot-core/src/remote/assets/panel'));
const IN_CI = Boolean(process.env.CI) && !/^(false|0)$/i.test(process.env.CI);

// The narrowest phone, the phone floor, a phone, each side of the two
// breakpoints, and a window wide enough that the list column's 30% is what
// sizes it. A fixed column fits at the extremes and overflows just above a
// breakpoint, so these are not interchangeable.
const WIDTHS = [280, 320, 390, 480, 700, 900, 1200, 1700];
// A renderer on a loaded Mac was measured doing nothing for 30-40s (load 100+,
// with the browser process answering throughout), so a wait has to outlast
// that. Every wait ends as soon as its condition holds, so this costs nothing
// unless the panel never gets there.
const WAIT_MS = 60_000;
const CALL_MS = 120_000;

const LONG_BRANCH = `feature/${'a-very-long-branch-name-without-spaces-'.repeat(3)}END`;
const AT = '2026-08-24T00:00:01Z';
const row = (id, fields) => ({
  session_id: id, live: false, addressable: true, status: 'idle', title: `session ${id}`,
  project_path: '/tmp/p', branch: 'main', messages: 16, models: ['claude-opus-5'],
  tokens: { input: 4000, output: 4000 }, last_ts: AT, has_error: false, unread: 0, ...fields,
});
const ROUTES = {
  '/api/health': { ok: true },
  '/api/me': { device: 'layout-check', passkeys: 0, server_version: '0.0.0' },
  '/api/sessions': {
    server_version: '0.0.0',
    sessions: [
      row('live', { live: true, has_error: true, branch: LONG_BRANCH }),
      row('no-path', { project_path: '' }),
      // A long project too: beside one, a warning placed after it falls off.
      row('long', {
        has_error: true, branch: LONG_BRANCH,
        project_path: '/tmp/a-project-with-a-name-long-enough-to-fill-a-narrow-row-and-more',
        title: `session long ${'a-title-with-no-room-'.repeat(12)}`,
      }),
      // More than a window of rows, so the list has to scroll to show them.
      ...Array.from({ length: 30 }, (_, i) => row(`history-${i}`, {})),
    ],
  },
  '/api/approvals': { approvals: [] },
  '/api/quick-prompts': { prompts: [] },
  '/api/accounts': {
    accounts: [
      {
        email: 'active@example.com', is_cli_active: true,
        // The long one first: a cell as wide as its label leaves the meters
        // unevenly spaced. In the middle, the spacing comes out even by symmetry.
        usage: {
          scoped: ['claude-fable-5-1-long-model-label', 'Haiku', 'Opus'].map((label) => (
            { kind: 'weekly_scoped', label, utilization: 40 }
          )),
        },
      },
      { email: 'a-long-address-that-has-to-truncate@example.com', is_cli_active: false },
    ],
  },
  '/api/inbound': { open: false },
};
// More than a window of it, so there is a last message to keep in view.
const TRANSCRIPT = {
  total: 60, next_cursor: 60,
  events: Array.from({ length: 60 }, (_, index) => ({ index, kind: 'user', ts: AT, text: 'hello' })),
};
// Slower than the panel's 4s poll, which is the case that used to starve.
const SLOW_MS = 6000;
const OUTAGE_MS = 6000;
const MANY = {
  server_version: '0.0.0',
  sessions: Array.from({ length: 120 }, (_, i) => row(`many-${i}`, { live: true })),
};
const TYPES = { '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html', '.woff2': 'font/woff2' };

function serve() {
  const stub = { missing: [], requests: 0, mode: null, dropping: null };
  const server = createServer((req, res) => {
    stub.requests++;
    const path = new URL(req.url, 'http://x').pathname;
    const json = (o) => { res.setHeader('content-type', 'application/json'); res.end(JSON.stringify(o)); };
    if (stub.mode === 'many' && path === '/api/sessions') return json(MANY);
    // For a while rather than a count of requests: Chrome quietly retries a
    // reset on a reused connection, and how many times is its own business.
    if (stub.mode === 'flaky' && path.endsWith('/transcript')) {
      stub.dropping ??= Date.now();
      if (Date.now() - stub.dropping < OUTAGE_MS) return req.socket.destroy();
    }
    if (stub.mode === 'slow' && path === '/api/approvals') return undefined; // Never answers.
    if (stub.mode === 'slow' && path === '/api/sessions') return setTimeout(() => json(ROUTES[path]), SLOW_MS);
    if (ROUTES[path]) return json(ROUTES[path]);
    if (path.endsWith('/transcript')) return json(TRANSCRIPT);
    if (path.startsWith('/api/')) { res.statusCode = 404; return json({}); }
    const rel = path === '/' ? 'index.html' : path.replace(/^\/panel\//, '').replace(/^\//, '');
    const file = normalize(join(OUT, rel));
    if (!file.startsWith(OUT + sep) || !existsSync(file)) {
      // A font that fails to load falls back and still lays out, so nothing
      // else would notice it. Root paths (/sw.js, the manifest, the icon) are
      // core's own assets and not this bundle; a worker would only get between
      // this check and the bundle.
      if (path.startsWith('/panel/')) stub.missing.push(path);
      res.statusCode = 404;
      return res.end();
    }
    res.setHeader('content-type', TYPES[extname(file)] ?? 'application/octet-stream');
    return res.end(readFileSync(file));
  });
  return new Promise((r) => server.listen(0, '127.0.0.1', () => r({ server, stub })));
}

function findChrome() {
  const named = process.env.CHROME_PATH;
  if (named) {
    if (existsSync(named)) return named;
    console.error(`layout-check: CHROME_PATH is ${named}, which does not exist`);
    return null;
  }
  const mac = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
  if (existsSync(mac)) return mac;
  for (const name of ['google-chrome', 'chromium', 'chromium-browser']) {
    const r = spawnSync('which', [name], { encoding: 'utf8' });
    if (r.status === 0) return r.stdout.trim();
  }
  return null;
}

/**
 * Files the bundle names and does not have. The walk fetches only what its
 * fixture needs: ASCII text never asks for a latin-ext font, and nothing opens a
 * lazy chunk, so those would otherwise go missing without a sound.
 */
function danglingRefs(root, extra = {}) {
  const css = readFileSync(join(root, 'panel.css'), 'utf8') + (extra.css ?? '');
  const fonts = [...css.matchAll(/url\(\/panel\/([^)]+)\)/g)].map((m) => m[1]);
  const js = readFileSync(join(root, 'panel.js'), 'utf8') + (extra.js ?? '');
  const fromMain = [...js.matchAll(/"\.\/(chunks\/[\w.-]+\.js)"/g)].map((m) => m[1]);
  const chunks = join(root, 'chunks');
  const texts = existsSync(chunks)
    ? readdirSync(chunks).filter((f) => f.endsWith('.js')).map((f) => readFileSync(join(chunks, f), 'utf8'))
    : [];
  const fromChunks = [...texts, extra.chunk ?? ''].flatMap((t) => (
    [...t.matchAll(/"\.\/([\w.-]+\.js)"/g)].map((m) => `chunks/${m[1]}`)
  ));
  return [...new Set([...fonts, ...fromMain, ...fromChunks])].filter((f) => !existsSync(join(root, f)));
}

/** The port Chrome wrote, or null while the file is still empty. */
const readPort = (file) => readFileSync(file, 'utf8').split('\n')[0].trim() || null;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function launch(binary) {
  const profile = mkdtempSync(join(tmpdir(), 'panel-layout-'));
  const proc = spawn(binary, [
    '--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
    '--no-first-run', '--disable-gpu', '--hide-scrollbars',
    // Nothing here is on screen, and the composer check waits on drawn frames.
    '--disable-renderer-backgrounding', '--disable-backgrounding-occluded-windows',
    '--disable-background-timer-throttling',
    'about:blank',
    // Ubuntu's runners forbid the sandbox's user namespaces. Only ever pointed
    // at this bundle on loopback.
    ...(IN_CI ? ['--no-sandbox'] : []),
  ], { stdio: 'ignore' });
  let ended = null;
  const exited = new Promise((r) => {
    proc.on('error', (e) => { ended = e.message; r(); });
    proc.on('exit', (code, signal) => { ended = `exit ${code ?? signal}`; r(); });
  });
  const stop = async () => {
    proc.kill();
    await Promise.race([exited, sleep(5000)]);
    if (!ended) proc.kill('SIGKILL');
    // Chrome can still be writing its profile as it exits.
    rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  };
  try {
    const portFile = join(profile, 'DevToolsActivePort');
    for (let i = 0; i < 200 && !existsSync(portFile) && !ended; i++) await sleep(100);
    if (!existsSync(portFile)) {
      // In a container Chrome usually cannot start its sandbox as this user.
      // CI=1 drops the sandbox.
      throw new Error(!ended ? 'Chrome did not open its debugging port'
        : ended.startsWith('exit') ? `Chrome stopped before it could be driven (${ended}); in a container, try CI=1`
          : `Chrome could not start: ${ended}`);
    }
    // Chrome creates the port file before writing it, and writes it before the
    // port answers. On a loaded machine each gap is long enough to lose one try.
    let targets = null;
    for (let tries = 20; !targets; tries--) {
      let why;
      const port = readPort(portFile);
      if (!port) why = 'its port file stayed empty';
      else {
        try {
          targets = await (await fetch(`http://127.0.0.1:${port}/json/list`, { signal: AbortSignal.timeout(5000) })).json();
        } catch (e) {
          why = `port ${port}: ${e.cause?.code ?? e.message}`;
        }
      }
      if (targets) break;
      if (!tries || ended) throw new Error(`could not reach Chrome (${ended ?? 'still running'}): ${why}`);
      await sleep(500);
    }
    const page = targets.find((t) => t.type === 'page');
    if (!page) throw new Error('Chrome has no page target');
    const ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((res, rej) => {
      const timer = setTimeout(() => rej(new Error(`debugger socket: no answer in ${CALL_MS}ms`)), CALL_MS);
      ws.onopen = () => { clearTimeout(timer); res(); };
      ws.onerror = () => { clearTimeout(timer); rej(new Error('debugger socket failed')); };
    });
    let seq = 0;
    const pending = new Map();
    const errors = [];
    ws.onmessage = (e) => {
      const m = JSON.parse(e.data);
      if (m.method === 'Runtime.exceptionThrown') errors.push(m.params.exceptionDetails.exception?.description ?? m.params.exceptionDetails.text);
      if (m.id && pending.has(m.id)) pending.get(m.id)(m);
    };
    // Chrome dying closes this socket. Without this every outstanding call
    // would wait forever, and only the job's own timeout would end the run.
    ws.onclose = () => {
      for (const settle of [...pending.values()]) settle({ error: { message: 'Chrome went away' } });
    };
    const cdp = (method, params = {}) => new Promise((res, rej) => {
      if (ws.readyState !== WebSocket.OPEN) { rej(new Error(`${method}: Chrome went away`)); return; }
      const id = ++seq;
      const timer = setTimeout(() => {
        pending.delete(id);
        rej(new Error(`${method}: no answer in ${CALL_MS}ms`));
      }, CALL_MS);
      pending.set(id, (m) => {
        clearTimeout(timer);
        pending.delete(id);
        if (m.error) rej(new Error(`${method}: ${m.error.message}`));
        else res(m.result);
      });
      ws.send(JSON.stringify({ id, method, params }));
    });
    return { cdp, errors, close: async () => { ws.onclose = null; ws.close(); await stop(); } };
  } catch (e) {
    await stop();
    throw e;
  }
}

/** Runs in the page. Returns failures as `id: detail`. */
const MEASURE = `(view) => {
  const bad = [];
  const fail = (id, detail) => bad.push(id + ': ' + detail);
  const shown = (el) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
  const panel = document.querySelector('.panel');
  if (!panel || !shown(panel)) { fail('not-rendered', 'no panel on screen'); return bad; }
  window.__fault?.();
  const gone = (colour) => colour === 'transparent' || colour === 'none' || /^rgba\\(.*, 0\\)$|\\/ (0|none)\\)$/.test(colour);
  // Boxes with a size can still draw nothing, and geometry cannot see that.
  const drawn = (el) => {
    if (getComputedStyle(el).visibility !== 'visible') return false;
    for (let e = el; e; e = e.parentElement) if (parseFloat(getComputedStyle(e).opacity) === 0) return false;
    return true;
  };
  // What a click would land on, which also sees clip-path and pointer-events.
  const reachable = (el) => {
    el.scrollIntoView({ block: 'center' });
    const r = el.getBoundingClientRect();
    return el.contains(document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2));
  };

  // Separators drawn by .dots: the first item on each line must start at the
  // box's own left edge, where its dot falls in the clipped strip.
  const sets = [...document.querySelectorAll('.dots')].filter(shown);
  for (const set of sets) {
    const box = set.getBoundingClientRect();
    const items = [...set.children].filter(shown);
    for (const item of [...set.children]) {
      if (!item.textContent.trim() && shown(item)) fail('dot-empty', 'an empty item takes the first place and shows its dot');
    }
    if (getComputedStyle(set).clipPath === 'none') fail('dot-clip', 'nothing clips the leading dot');
    if (getComputedStyle(set).flexWrap !== 'wrap') fail('dot-wrap', 'details cannot wrap between items');
    if (set.matches('.dots-one-line') && box.height > parseFloat(getComputedStyle(set).lineHeight) * 1.1) fail('row-height', 'a list row has ' + Math.round(box.height) + 'px of details');
    if (!drawn(set)) fail('invisible', 'a row of details');
    if (gone(getComputedStyle(set).color)) fail('invisible', 'details in a transparent colour');
    let top = null;
    for (const item of items) {
      const s = getComputedStyle(item);
      const dot = getComputedStyle(item, '::before');
      const r = item.getBoundingClientRect();
      if (!drawn(item)) fail('invisible', 'an item: its opacity');
      if (gone(s.webkitTextFillColor)) fail('invisible', 'an item in a transparent colour');
      if (s.position !== 'relative') fail('dot-position', 'the dot is not anchored to its item');
      if (parseFloat(s.paddingLeft) < 2) fail('dot-width', 'no room for the dot');
      if (Math.abs(parseFloat(dot.width) - parseFloat(s.paddingLeft)) > 1) fail('dot-width', 'the dot is ' + dot.width + ' in a ' + s.paddingLeft + ' strip');
      if (!dot.content.includes('·')) fail('dot-position', 'item has no dot');
      if (dot.opacity === '0' || dot.visibility !== 'visible' || dot.display === 'none') fail('dot-hidden', 'the dot does not show');
      if (gone(dot.color)) fail('dot-hidden', 'the dot is a transparent colour');
      if (top === null || r.top > top + 1) {
        if (Math.abs(r.left - box.left) > 1) fail('dot-line-start', 'a line starts ' + Math.round(r.left - box.left) + 'px in');
      }
      top = r.top;
      if (r.right > box.right + 1) fail('dot-overflow', '"' + item.textContent.slice(0, 20) + '" runs past its box');
    }
  }

  // Separators written into text: one that opens a line, or stands alone.
  const walker = document.createTreeWalker(panel, NodeFilter.SHOW_TEXT);
  const nodes = [];
  while (walker.nextNode()) if (walker.currentNode.parentElement && shown(walker.currentNode.parentElement)) nodes.push(walker.currentNode);
  const block = (n) => { let e = n.parentElement; while (e && getComputedStyle(e).display.startsWith('inline')) e = e.parentElement; return e; };
  const rect = (n, i) => { const g = document.createRange(); g.setStart(n, i); g.setEnd(n, i + 1); return g.getBoundingClientRect(); };
  nodes.forEach((node, at) => {
    for (let i = 0; i < node.data.length; i++) {
      if (node.data[i] !== '·') continue;
      let before = null;
      for (let n = at, from = i - 1; n >= 0 && !before; n--, from = n >= 0 ? nodes[n].data.length - 1 : -1) {
        if (block(nodes[n]) !== block(node)) break;
        for (let k = from; k >= 0; k--) if (nodes[n].data[k].trim() && nodes[n].data[k] !== '\\u00a0') { before = rect(nodes[n], k); break; }
      }
      if (!before || rect(node, i).top > before.top + 2) fail('dot-line-start', '"' + node.data.slice(0, 24) + '"');
    }
  });

  // Tabs: the badge beside its icon; the active icon a whole shape in accent.
  const tab = (name) => [...document.querySelectorAll('button')].find((b) => b.getAttribute('aria-label') === name);
  const sessions = tab('Sessions');
  if (!sessions) { fail('not-rendered', 'no tabs'); return bad; }
  for (const name of ['Sessions', 'Accounts', 'Settings']) {
    const svg = tab(name)?.querySelector('svg');
    if (!svg) { fail('tab-icon', name + ' has no icon'); continue; }
    // The active icon is filled and has no stroke; an idle one is the reverse.
    const paint = getComputedStyle(svg);
    if (!shown(svg) || !drawn(svg) || (gone(paint.stroke) && gone(paint.fill))) fail('invisible', 'the ' + name + ' icon');
  }
  const badge = sessions.querySelector('span.mono');
  if (!badge || !badge.textContent.trim()) fail('fixture', 'a live session should badge the tab');
  else {
    if (!drawn(badge) || gone(getComputedStyle(badge).color)) fail('invisible', 'the badge');
    const b = badge.getBoundingClientRect();
    const i = sessions.querySelector('svg').getBoundingClientRect();
    const label = [...sessions.querySelectorAll('span')].find((s) => !s.children.length && s.textContent === 'Sessions')?.getBoundingClientRect();
    // The widest count may take a corner of the icon, and no more.
    const tucked = badge.textContent.length > 2 ? 4.5 : 0.5;
    if (b.left < i.right - tucked && b.right > i.left && b.top < i.bottom && b.bottom > i.top) fail('badge-overlap', Math.round(i.right - b.left) + 'px over the icon');
    if (b.right > innerWidth) fail('badge-overlap', 'badge leaves the window');
    // The bar or rail holding the tabs, whose edge the rail's divider sits on. The
    // halo counts, because it paints the page colour over whatever it crosses.
    const edge = sessions.parentElement.getBoundingClientRect().right;
    if (b.right + parseFloat(getComputedStyle(badge).getPropertyValue('--badge-halo')) > edge + 0.5) fail('badge-tab', 'the badge ends at ' + Math.round(b.right) + 'px with the edge at ' + Math.round(edge) + 'px');
    if (view === 'many' && badge.textContent !== '99+') fail('fixture', 'a hundred live sessions should read 99+, not ' + badge.textContent);
    // Beside the icon also means level with it, and clear of the tab's name.
    if (b.top >= i.bottom || b.bottom <= i.top || b.left - i.right > 6) fail('badge-position', 'the badge has left its icon');
    if (label && b.left < label.right && b.right > label.left && b.top < label.bottom && b.bottom > label.top) fail('badge-position', 'the badge sits on the tab name');
    if (b.top < 0 || b.bottom > innerHeight) fail('badge-position', 'badge leaves the window');
  }
  const probe = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  probe.style.stroke = 'var(--ac)';
  panel.append(probe);
  const accent = getComputedStyle(probe).stroke;
  probe.remove();
  const active = document.querySelector('button[aria-current]');
  const icon = active && active.querySelector('svg');
  if (!icon || !icon.children.length) fail('tab-icon', 'no active icon');
  else if (active.getAttribute('aria-label') === 'Settings') {
    const s = getComputedStyle(icon);
    const idle = getComputedStyle(sessions.querySelector('svg'));
    if (s.stroke !== accent) fail('tab-icon', 'stroke ' + s.stroke);
    if (s.fill !== 'none') fail('tab-icon', 'sliders have nothing to fill');
    if (parseFloat(s.strokeWidth) <= parseFloat(idle.strokeWidth)) fail('tab-icon', 'no heavier than an idle icon');
  } else if (getComputedStyle(icon).fill !== accent) fail('tab-icon', 'fill ' + getComputedStyle(icon).fill);

  // Anything past an ancestor that clips it adds no scroll to the page, so
  // only this comparison sees it.
  for (const el of panel.querySelectorAll('*')) {
    if (el.closest('svg') && el.tagName !== 'svg') continue;
    if (!shown(el)) continue;
    const r = el.getBoundingClientRect();
    for (let a = el.parentElement; a && a !== document.documentElement; a = a.parentElement) {
      const o = getComputedStyle(a).overflowX;
      if (o === 'visible') continue;
      if (o === 'auto' || o === 'scroll') break;
      const c = a.getBoundingClientRect();
      if (r.right > c.right + 1 || r.left < c.left - 1) {
        if (getComputedStyle(el).position !== 'absolute' || el.textContent.trim()) fail('clipped', (el.getAttribute('role') || el.tagName) + ' "' + el.textContent.trim().slice(0, 24) + '"');
      }
      break;
    }
    if (el.children.length || !el.textContent.trim()) continue;
    for (let a = el.parentElement; a && a !== document.documentElement; a = a.parentElement) {
      const o = getComputedStyle(a).overflowY;
      if (o === 'visible') continue;
      if (o === 'auto' || o === 'scroll') break;
      const c = a.getBoundingClientRect();
      // Moved whole to the line a one-line row hides, which is the point of it.
      if (a.matches('.dots-one-line') && r.top >= c.bottom - 1) break;
      if (r.bottom > c.bottom + 1 || r.top < c.top - 1) fail('cut-vertically', '"' + el.textContent.trim().slice(0, 24) + '"');
      break;
    }
  }
  // Text cut short has to be readable somewhere. The composer's placeholder is
  // the one exception: the reason it shortens is written out below the field.
  for (const el of panel.querySelectorAll('*')) {
    if (el.tagName === 'TEXTAREA' || !shown(el) || getComputedStyle(el).textOverflow !== 'ellipsis') continue;
    if (el.scrollWidth > el.clientWidth && !el.closest('[title]')) fail('no-title', '"' + el.textContent.slice(0, 24) + '" is cut short with no way to read it');
  }
  if (document.documentElement.scrollWidth > innerWidth) fail('page-overflow', 'the page scrolls sideways');

  // The stub's hard cases must have been on screen, or none of this ran.
  if (view === 'sessions') {
    const items = [...document.querySelectorAll('.dots > *')];
    if (sets.length < 32) fail('fixture', sets.length + ' rows of details');
    if (!items.some((e) => !e.textContent.trim())) fail('fixture', 'the empty project path never reached a row');
    if (!items.some((e) => e.textContent.endsWith('END') && shown(e))) fail('fixture', 'the long branch never reached a row');
    // A warning that drops off a full row is a warning nobody gets.
    const warning = [...document.querySelectorAll('.dots-one-line > *')].find((e) => e.textContent === 'had errors');
    if (!warning) fail('fixture', 'no list row carries a warning');
    else if (warning.getBoundingClientRect().bottom > warning.parentElement.getBoundingClientRect().bottom + 1) fail('row-warning', 'the warning fell off its row');
    // A long project name shortens beside the warning, rather than taking itself and everything after it off the row.
    for (const r of document.querySelectorAll('.dots-one-line')) {
      const project = r.querySelector(':scope > .dots-project');
      const edge = r.getBoundingClientRect().bottom + 1;
      if (project?.textContent && project.getBoundingClientRect().bottom > edge) fail('row-project', 'a project fell off its row');
      const seen = [...r.children].filter((e) => e.getBoundingClientRect().bottom <= edge && e.getBoundingClientRect().width);
      const spare = r.getBoundingClientRect().right - Math.max(...seen.map((e) => e.getBoundingClientRect().right));
      if (project && project.scrollWidth > project.clientWidth && spare > 2) fail('row-room', 'a shortened project left ' + Math.round(spare) + 'px of its line empty');
    }

    // The list has to scroll to its end, or the rows past the window are gone.
    const last = sets[sets.length - 1];
    const list = last?.closest('.sc');
    if (!list || !['auto', 'scroll'].includes(getComputedStyle(list).overflowY) || list.scrollHeight <= list.clientHeight) fail('scroll', 'the list does not scroll');
    else {
      list.scrollTop = list.scrollHeight;
      const r = last.getBoundingClientRect();
      if (r.bottom > innerHeight + 1 || r.top < 0) fail('scroll', 'the last row stays off screen');
      list.scrollTop = 0;
    }
    if (innerWidth >= 900) {
      const hint = [...panel.querySelectorAll('p')].find((p) => p.textContent === 'Pick a session');
      if (!hint) fail('fixture', 'the wide layout shows no empty detail pane');
      else if (list) {
        let col = list;
        while (col.parentElement && !col.parentElement.contains(hint)) col = col.parentElement;
        const w = col.getBoundingClientRect().width;
        const stage = col.parentElement.getBoundingClientRect().width;
        if (w + 1 < stage * 0.3) fail('list-width', Math.round(w) + ' of ' + Math.round(stage));
      }
    }
  }
  if (view === 'settings') {
    if (document.querySelectorAll('[role=tablist]').length < 2) fail('fixture', 'Theme and Tool calls should both show');
    for (const b of document.querySelectorAll('[role=tablist] button')) if (!reachable(b)) fail('covered', 'a Settings choice');
  }
  if (view === 'accounts') {
    if (!panel.textContent.includes('active@example.com')) fail('fixture', 'no account on screen');
    const cells = [...(document.querySelector('[style*="auto-fill"]')?.children ?? [])];
    if (cells.length < 3) fail('fixture', 'the account shows ' + cells.length + ' meters');
    const middles = cells.map((c) => { const r = c.firstElementChild.getBoundingClientRect(); return { top: r.top, x: r.left + r.width / 2 }; });
    const gaps = middles.slice(1).flatMap((m, i) => (m.top === middles[i].top ? [m.x - middles[i].x] : []));
    if (gaps.some((g) => Math.abs(g - gaps[0]) > 1)) fail('meter-line', 'meters ' + gaps.map(Math.round).join(', ') + 'px apart');
    // Measured on the label: the ring's rotated drawing counts as overflow too.
    for (const c of cells) if (c.lastElementChild.getBoundingClientRect().right > c.getBoundingClientRect().right + 1) fail('meter-line', 'a label runs out of its cell');
  }
  if (view === 'thread' || view === 'thread-no-path') {
    const t = document.querySelector('textarea[aria-label="Message this session"]');
    if (!t) fail('fixture', 'the thread has no composer');
    else {
      const s = getComputedStyle(t);
      const ph = getComputedStyle(t, '::placeholder');
      const inner = t.clientHeight - parseFloat(s.paddingTop) - parseFloat(s.paddingBottom);
      const line = parseFloat(s.lineHeight);
      const r = t.getBoundingClientRect();
      if (t.value) fail('fixture', 'the composer should be empty');
      if (!drawn(t)) fail('invisible', 'the composer');
      if (gone(s.webkitTextFillColor)) fail('invisible', 'the composer text');
      if (gone(ph.color)) fail('invisible', 'the composer hint');
      if (!reachable(t)) fail('covered', 'the composer');
      if (s.whiteSpace !== 'nowrap' || s.overflowX !== 'hidden' || s.textOverflow !== 'ellipsis') fail('placeholder', 'the field does not hold one line');
      if (ph.whiteSpace !== 'nowrap' || ph.overflowX !== 'hidden' || ph.textOverflow !== 'ellipsis') fail('placeholder', 'the placeholder does not hold one line');
      if (inner + 1 < line) fail('placeholder', inner + ' high for a ' + line + ' line');
      // Halfway between one line and two: two rows round to just under 2x.
      if (inner >= line * 1.5) fail('placeholder', 'an empty composer is ' + inner + ' high');
      if (r.bottom > innerHeight + 1 || r.top < 0) fail('composer-offscreen', 'the composer is outside the window');
    }
    if (view === 'thread') {
      const copy = document.querySelector('button[aria-label="Copy the project path"]');
      if (!copy) fail('fixture', 'the thread offers no way to copy its path');
      else {
        const r = copy.getBoundingClientRect();
        if (Math.min(r.width, r.height) < 44) fail('tap-target', 'Copy path is ' + Math.round(r.width) + 'x' + Math.round(r.height));
        if (!reachable(copy)) fail('covered', 'Copy path');
      }
    }
    if (view === 'thread-no-path' && !sets.some((d) => [...d.children].some((c) => !c.textContent.trim()))) fail('fixture', 'the thread shows no empty project path');
  }
  return bad;
}`;

/** Types into the composer, which only a real input event can test. Restores it. */
const TYPED = `async () => {
  const bad = [];
  const fail = (id, detail) => bad.push(id + ': ' + detail);
  const t = document.querySelector('textarea[aria-label="Message this session"]');
  if (!t) return ['fixture: the thread has no composer to type into'];
  const write = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
  // Counted in frames rather than time, because a resize is only reported on
  // one. A page that stops drawing says so here, rather than hanging the run.
  const frame = () => new Promise((resolve, reject) => {
    requestAnimationFrame(resolve);
    setTimeout(() => reject(new Error('no frame in 5s')), 5000);
  });
  const type = async (text) => {
    write.call(t, text);
    t.dispatchEvent(new Event('input', { bubbles: true }));
    for (let i = 0; i < 3; i++) await frame();
  };
  window.__typing?.();
  const said = [...document.querySelectorAll('.panel *')].find((e) => !e.children.length && e.textContent === 'hello');
  let list = said?.parentElement;
  while (list && !['auto', 'scroll'].includes(getComputedStyle(list).overflowY)) list = list.parentElement;
  if (!list || list.scrollHeight <= list.clientHeight) fail('fixture', 'the transcript does not scroll');
  const s = getComputedStyle(t);
  const line = parseFloat(s.lineHeight);
  const inner = () => t.clientHeight - parseFloat(s.paddingTop) - parseFloat(s.paddingBottom);
  try {
    await type('one\\ntwo\\nthree');
    if (inner() < line * 2.5) fail('composer-grow', 'three lines in ' + inner() + 'px');
    if (t.scrollHeight > t.clientHeight + 1) fail('composer-grow', 'three lines have to scroll');
    if (list && list.scrollHeight - list.scrollTop - list.clientHeight > 4) fail('composer-pin', 'the last message went behind the composer');
    await type('x\\n'.repeat(60));
    const cap = parseFloat(s.maxHeight);
    if (!(t.offsetHeight <= cap + 1)) fail('composer-cap', t.offsetHeight + 'px with a cap of ' + s.maxHeight);
    if (t.scrollHeight <= t.clientHeight) fail('composer-cap', 'sixty lines have nowhere to scroll');
    if (t.getBoundingClientRect().top < 0) fail('composer-cap', 'the composer grew out of the window');
    await type('x'.repeat(4000));
    if (t.scrollWidth > t.clientWidth + 1) fail('composer-wrap', 'unbroken text runs off the side');
    await type('');
    if (inner() >= line * 1.5) fail('composer-shrink', 'emptied, the composer stayed ' + inner() + 'px');
  } catch (e) {
    fail('no-frame', e.message);
  } finally {
    await type('').catch(() => {});
  }
  return bad;
}`;

// What has to be in the DOM before a view can be measured. Existence rather
// than visibility: a hidden panel must reach the measure and fail by name.
// A document just navigated to has no body until the parser reaches one.
const TEXT = `(document.body?.textContent ?? '')`;
const COMPOSER = `!!document.querySelector('textarea[aria-label="Message this session"]')`;
const READY = {
  sessions: `${TEXT}.includes('session no-path') && ${TEXT}.includes('END')`,
  many: `${TEXT}.includes('session many-119')`,
  accounts: `${TEXT}.includes('active@example.com')`,
  settings: `document.querySelectorAll('[role=tablist]').length >= 2 && ${TEXT}.includes('Closed') && !${TEXT}.includes('Checking')`,
  thread: `${COMPOSER} && [...document.querySelectorAll('.dots.selectable')].some((d) => d.textContent.includes('END')) && ${TEXT}.includes('hello')`,
  'thread-no-path': `${COMPOSER} && [...document.querySelectorAll('.dots.selectable')].some((d) => [...d.children].some((c) => !c.textContent.trim()))`,
};

class NotRendered extends Error {}

async function walk(chrome, origin, stub, fault, want) {
  const { cdp, errors } = chrome;
  const failures = [];
  const js = async (expression) => {
    const r = await cdp('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }).catch((e) => {
      throw new Error(`${e.message}, running: ${expression.slice(0, 40).replace(/\s+/g, ' ')}`);
    });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
    return r.result.value;
  };
  const until = async (expression, what) => {
    const end = Date.now() + WAIT_MS;
    const before = stub.requests;
    let slowest = 0;
    while (Date.now() < end) {
      const asked = Date.now();
      if (await js(expression)) return;
      slowest = Math.max(slowest, Date.now() - asked);
      await sleep(50);
    }
    // A wait that ran out on a loaded machine and a panel that is really blank
    // read alike without this. It is evidence and not a verdict: a broken panel
    // can make no requests, and a blank one still fetches its own files.
    const page = await js(`${TEXT}.replace(/\\s+/g, ' ').slice(0, 160)`);
    throw new NotRendered(`${what}; ${WAIT_MS / 1000}s, ${stub.requests - before} requests and a slowest page answer of ${Math.round(slowest / 1000)}s later the page read: "${page}"${errors.length ? `; errors: ${errors.join(' | ').slice(0, 300)}` : ''}`);
  };
  // Ready says the data arrived; this says React has finished with it.
  const quiet = async () => {
    const end = Date.now() + WAIT_MS;
    let same = 0;
    let previous = null;
    while (Date.now() < end && same < 3) {
      const now = await js(`(document.querySelector('.panel')?.innerHTML.length ?? 0) + ':' + document.documentElement.scrollHeight`);
      same = now === previous ? same + 1 : 0;
      previous = now;
      await sleep(50);
    }
    if (same < 3) throw new NotRendered('the panel never settled');
    await js('document.fonts.ready.then(() => true)');
  };
  const click = async (expression, what) => {
    await until(`!!(${expression})`, what);
    // Where a finger lands, not el.click(): a control under an overlay or with
    // pointer-events off is dead to a person and fine to a script.
    const hit = await js(`(() => {
      const el = ${expression};
      el.scrollIntoView({ block: 'center' });
      const r = el.getBoundingClientRect();
      const at = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      if (!el.contains(at)) return (at?.tagName ?? 'nothing') + ' ' + (at?.className ?? '');
      el.click();
      return null;
    })()`);
    if (hit) throw new NotRendered(`${what}: covered by ${hit}`);
  };
  const tab = (name) => `[...document.querySelectorAll('button')].find((b) => b.getAttribute('aria-label') === ${JSON.stringify(name)})`;
  const session = (title) => `[...document.querySelectorAll('.panel button, .panel [role=button], .panel div')].filter((e) => e.textContent.includes(${JSON.stringify(title)}) && e.querySelector('.dots')).at(-1)?.querySelector('.dots')`;
  const finished = () => want && failures.some((f) => f.includes(want));
  const measure = async (width, view) => {
    await until(READY[view], `${view} never loaded`);
    await quiet();
    for (const f of await js(`(${MEASURE})(${JSON.stringify(view)})`)) failures.push(`${width} ${view} ${f}`);
    if (view === 'thread') for (const f of await js(`(${TYPED})()`)) failures.push(`${width} ${view} ${f}`);
    for (const path of stub.missing.splice(0)) failures.push(`${width} ${view} asset-missing: ${path}`);
  };
  const init = `localStorage.setItem('claudepot.token', 'layout-check');
    ${fault?.css ? `addEventListener('DOMContentLoaded', () => { const s = document.createElement('style'); s.textContent = ${JSON.stringify(fault.css)}; document.head.append(s); });` : ''}
    ${fault?.page ?? ''}`;
  const { identifier } = await cdp('Page.addScriptToEvaluateOnNewDocument', { source: init });
  try {
    for (const width of WIDTHS) {
      await cdp('Emulation.setDeviceMetricsOverride', { width, height: 800, deviceScaleFactor: 1, mobile: false });
      await cdp('Page.navigate', { url: origin });
      await sleep(50); // The old document must be gone before READY reads one.
      const steps = [
        [null, 'sessions'],
        [() => click(tab('Accounts'), 'no Accounts tab'), 'accounts'],
        [() => click(tab('Settings'), 'no Settings tab'), 'settings'],
        [async () => {
          await click(tab('Sessions'), 'no Sessions tab');
          await click(session('session live'), 'no live row');
        }, 'thread'],
        [async () => {
          if (width < 900) await click(`document.querySelector('[aria-label="Back"]')`, 'no way back');
          await click(session('session no-path'), 'no row without a path');
        }, 'thread-no-path'],
      ];
      for (const [act, view] of steps) {
        try {
          await act?.();
          await measure(width, view);
        } catch (e) {
          if (!(e instanceof NotRendered)) throw e;
          // The next view starts from this one, so there is nothing to walk.
          failures.push(`${width} ${view} not-rendered: ${e.message}`);
          return failures;
        }
        if (finished()) return failures;
      }
    }
    // The two states the fixture cannot hold at once. `many`: a count at its
    // widest, at the width where the rail leaves it least room. `slow`: a list
    // slower than the poll beside an approvals request that never answers,
    // which must still arrive. `flaky`: the thread's first read fails, and the
    // thread must recover without being reopened.
    const passes = [
      ['many', 1200, 'many'],
      ['slow', 390, 'sessions'],
      ['flaky', 390, 'thread', () => click(session('session live'), 'no live row')],
    ];
    for (const [mode, width, view, act] of passes) {
      stub.mode = mode;
      stub.dropping = null;
      try {
        await cdp('Emulation.setDeviceMetricsOverride', { width, height: 800, deviceScaleFactor: 1, mobile: false });
        await cdp('Page.navigate', { url: origin });
        await sleep(50);
        await act?.();
        await measure(`${width} ${mode}`, view);
      } catch (e) {
        if (!(e instanceof NotRendered)) throw e;
        failures.push(`${width} ${mode} not-rendered: ${e.message}`);
        return failures;
      } finally {
        stub.mode = null;
      }
      if (finished()) return failures;
    }
    return failures;
  } finally {
    // After a Chrome that died this cannot answer, and would hide the real error.
    await cdp('Page.removeScriptToEvaluateOnNewDocument', { identifier }).catch(() => {});
    for (const e of errors.splice(0)) failures.push(`console ${e}`);
  }
}

const BADGE = 'button[aria-label="Sessions"] span[style*="left: 100%"]';
const SETTINGS_ICON = 'button[aria-label="Settings"][aria-current] svg';
const run = (body) => `window.__fault = () => { ${body} };`;
const drop = (selector) => run(`document.querySelectorAll(${JSON.stringify(selector)}).forEach((e) => e.remove());`);
const dropEmptyIn = (selector) => run(`for (const e of document.querySelectorAll(${JSON.stringify(selector)})) if (!e.textContent.trim()) e.remove();`);
// Each names the text of one failure, so a clause that stops working cannot
// hide behind another with the same id. Every fail() in MEASURE has a row.
const FAULTS = [
  ['not-rendered: no panel on screen', { css: '.panel { display: none !important; }' }],
  ['dot-empty:', { css: '.dots > :empty { display: inline !important; }' }],
  ['dot-clip:', { css: '.dots { clip-path: none !important; }' }],
  ['dot-wrap:', { css: '.dots { flex-wrap: nowrap !important; }' }],
  ['invisible: a row of details', { css: '.dots { opacity: 0 !important; }' }],
  ['invisible: details in a transparent colour', { css: '.dots { color: transparent !important; }' }],
  ['invisible: an item in a transparent colour', { css: '.dots > * { color: transparent !important; } .dots > *::before { color: gray !important; }' }],
  ['invisible: an item in a transparent colour', { css: '.dots > * { -webkit-text-fill-color: transparent !important; }' }],
  ['invisible: an item in a transparent colour', { css: '.dots > * { color: oklch(.5 .1 30 / none) !important; }' }],
  ['invisible: an item:', { css: '.dots > * { opacity: 0 !important; }' }],
  ['dot-position: the dot is not anchored', { css: '.dots > * { position: static !important; }' }],
  ['dot-width: no room', { css: '.dots { --dot-w: 0px !important; }' }],
  ['dot-width: the dot is', { css: '.dots > *::before { width: 1px !important; }' }],
  ['dot-position: item has no dot', { css: '.dots > *::before { content: "x" !important; }' }],
  ['dot-hidden: the dot does not show', { css: '.dots > *::before { opacity: 0 !important; }' }],
  ['dot-hidden: the dot does not show', { css: '.dots > *::before { visibility: hidden !important; }' }],
  ['dot-hidden: the dot is a transparent colour', { css: '.dots > *::before { color: transparent !important; }' }],
  ['dot-line-start: a line starts', { css: '.dots > * { margin-left: 8px !important; }' }],
  ['dot-overflow:', { css: '.dots > * { flex: 0 0 5000px !important; }' }],
  // A separator as text, in a box narrow enough to put it on its own line.
  ['dot-line-start: "aaaa', { page: run(`if (document.getElementById('f')) return; const d = document.createElement('div'); d.id = 'f'; d.style.width = '1ch'; d.textContent = 'aaaa · bbbb'; document.querySelector('.panel').prepend(d);`) }],
  ['not-rendered: no tabs', { page: run(`document.querySelector('button[aria-label="Sessions"]')?.removeAttribute('aria-label');`) }],
  ['tab-icon: Accounts has no icon', { page: drop('button[aria-label="Accounts"] svg') }],
  ['invisible: the Accounts icon', { css: 'button[aria-label="Accounts"] svg { opacity: 0 !important; }' }],
  ['invisible: the Accounts icon', { css: 'button[aria-label="Accounts"] svg { stroke: transparent !important; }' }],
  ['invisible: the Accounts icon', { css: 'button[aria-label="Accounts"] svg { stroke: none !important; }' }],
  ['invisible: the Accounts icon', { css: 'button[aria-label="Accounts"] svg { display: none !important; }' }],
  ['fixture: a live session should badge', { page: drop('button[aria-label="Sessions"] span.mono') }],
  ['invisible: the badge', { css: `button[aria-label="Sessions"] span.mono { color: transparent !important; }` }],
  ['invisible: the badge', { css: 'button[aria-label="Sessions"] span.mono { opacity: 0 !important; }' }],
  ['badge-position: the badge has left its icon', { css: `${BADGE} { left: calc(100% + 20px) !important; }` }],
  ['px over the icon', { css: `${BADGE} { left: 0 !important; }` }],
  ['badge-overlap: badge leaves the window', { css: `${BADGE} { left: 5000px !important; }` }],
  ['badge-position: the badge has left its icon', { css: `${BADGE} { top: -60px !important; }` }],
  ['badge-position: the badge sits on the tab name', { css: `${BADGE} { top: 30px !important; left: 0 !important; }` }],
  ['badge-position: badge leaves the window', { css: `${BADGE} { top: -5000px !important; }` }],
  ['tab-icon: no active icon', { page: run(`document.querySelector('button[aria-current] svg')?.replaceChildren();`) }],
  ['tab-icon: stroke', { css: `${SETTINGS_ICON} { stroke: gray !important; }` }],
  ['tab-icon: sliders have nothing to fill', { css: `${SETTINGS_ICON} { fill: gray !important; }` }],
  ['tab-icon: no heavier', { css: `${SETTINGS_ICON} { stroke-width: 0.5px !important; }` }],
  ['tab-icon: fill', { css: 'button[aria-current] svg { fill: gray !important; }' }],
  // The widest count is in play only in the `many` pass, after every width.
  ['fixture: a hundred live sessions', { page: run(`if (document.body.textContent.includes('session many-119')) document.querySelector('button[aria-label="Sessions"] span.mono').textContent = '120';`) }],
  ['badge-tab:', { css: 'button[aria-label="Sessions"] span[style*="calc(100%"] { left: 100% !important; }' }],
  ['px over the icon', { css: 'button[aria-label="Sessions"] span[style*="calc(100%"] { left: calc(100% - 12px) !important; }' }],
  ['row-height:', { css: '.dots-one-line { max-height: none !important; }' }],
  ['cut-vertically:', { css: '.dots-one-line { max-height: 8px !important; }' }],
  ['row-warning:', { css: '.dots-one-line > :first-child { order: 99 !important; }' }],
  ['row-project:', { css: '.dots-one-line > .dots-project { flex: 0 0 auto !important; max-width: none !important; }' }],
  ['row-room:', { css: '.dots-one-line > .dots-project { flex: 0 0 50% !important; }' }],
  ['fixture: no list row carries a warning', { page: run(`for (const e of document.querySelectorAll('.dots-one-line > *')) if (e.textContent === 'had errors') e.remove();`) }],
  ['no-title:', { page: run(`document.querySelectorAll('.panel [title]').forEach((e) => e.removeAttribute('title'));`) }],
  ['fixture: the account shows', { page: run(`document.querySelector('[style*="auto-fill"]')?.replaceChildren();`) }],
  ['meter-line: meters', { css: '[style*="auto-fill"] { display: flex !important; flex-wrap: wrap !important; }' }],
  ['meter-line: a label', { css: '[style*="auto-fill"] > * > :nth-child(2) { white-space: nowrap !important; overflow-wrap: normal !important; }' }],
  ['fixture: the thread offers no way', { page: drop('button[aria-label="Copy the project path"]') }],
  ['tap-target:', { css: 'button[aria-label="Copy the project path"] { width: 28px !important; height: 28px !important; margin: 0 !important; }' }],
  ['covered: Copy path', { css: 'button[aria-label="Copy the project path"] { pointer-events: none !important; }' }],
  ['fixture: the transcript does not scroll', { page: `window.__typing = () => { for (const e of document.querySelectorAll('.panel *')) if (e.textContent === 'hello') e.textContent = 'gone'; };` }],
  ['composer-grow: three lines in', { css: '.one-line-hint { height: 4px !important; min-height: 0 !important; }' }],
  ['composer-grow: three lines have to scroll', { css: '.one-line-hint { height: var(--ctl-lg) !important; }' }],
  // The growing composer no longer pulls the last message back into view.
  ['composer-pin:', { page: 'delete window.ResizeObserver;' }],
  ['px with a cap of', { css: '.one-line-hint { max-height: none !important; }' }],
  ['composer-cap: sixty lines', { css: '.one-line-hint { max-height: none !important; }' }],
  ['composer-cap: the composer grew', { css: '.one-line-hint { transform: translateY(-2000px) !important; }' }],
  ['no-frame:', { page: 'window.requestAnimationFrame = () => 0;' }],
  // Costs a minute, like the other two that run out a wait.
  ['flaky not-rendered: thread never loaded', { page: `{ const real = window.setTimeout; window.setTimeout = (f, ms, ...rest) => (ms === 4000 ? 0 : real(f, ms, ...rest)); }` }],
  ['composer-wrap:', { css: '.one-line-hint { white-space: pre !important; }' }],
  // A field that measures itself without first letting go of its height.
  ['composer-shrink:', { page: `{ const d = Object.getOwnPropertyDescriptor(Element.prototype, 'scrollHeight'); let most = 0; Object.defineProperty(HTMLTextAreaElement.prototype, 'scrollHeight', { get() { most = Math.max(most, d.get.call(this)); return most; } }); }` }],
  // What the poll used to do: every request abandoned at the next tick.
  ['slow not-rendered: sessions never loaded', { page: `{ const real = window.fetch; window.fetch = (path, init = {}) => { if (path !== '/api/sessions') return real(path, init); const c = new AbortController(); setTimeout(() => c.abort(), 4000); return real(path, { ...init, signal: c.signal }); }; }` }],
  ['clipped: ', { css: 'div:has(> [role=tablist]) { flex-wrap: nowrap !important; }' }],
  ['cut-vertically:', { css: '.dots { overflow-y: hidden !important; max-height: 2px !important; }' }],
  ['page-overflow:', { page: run(`if (document.getElementById('f')) return; const d = document.createElement('div'); d.id = 'f'; d.style.cssText = 'position:absolute;width:9000px;height:1px'; document.body.append(d);`) }],
  ['fixture: 5 rows of details', { page: run(`[...document.querySelectorAll('.dots')].slice(5).forEach((e) => e.remove());`) }],
  ['fixture: the empty project path never reached a row', { page: dropEmptyIn('.dots > *') }],
  // The hard rows must have shown, not merely be in the document.
  ['fixture: the long branch never reached a row', { page: run(`for (const e of document.querySelectorAll('.dots > *')) if (e.textContent.endsWith('END')) e.style.display = 'none';`) }],
  ['scroll: the list does not scroll', { css: '.sc { overflow: hidden !important; }' }],
  ['scroll: the last row stays off screen', { css: '.sc { padding-bottom: 5000px !important; }' }],
  ['fixture: the wide layout shows no empty detail pane', { page: run(`for (const p of document.querySelectorAll('.panel p')) if (p.textContent === 'Pick a session') p.remove();`) }],
  ['list-width:', { css: 'div[style*="--pane-list"] { width: 200px !important; }' }],
  ['fixture: Theme and Tool calls', { page: run(`document.querySelector('[role=tablist]')?.remove();`) }],
  ['fixture: no account on screen', { page: run(`const w = document.createTreeWalker(document.querySelector('.panel'), NodeFilter.SHOW_TEXT); while (w.nextNode()) w.currentNode.data = w.currentNode.data.replace('active@example.com', '');`) }],
  ['fixture: the thread has no composer', { page: drop('textarea[aria-label="Message this session"]') }],
  ['fixture: the composer should be empty', { page: run(`const t = document.querySelector('textarea[aria-label="Message this session"]'); if (t) t.value = 'x';`) }],
  ['invisible: the composer', { css: '.one-line-hint { visibility: hidden !important; }' }],
  ['invisible: the composer', { css: '.one-line-hint { opacity: 0 !important; }' }],
  ['invisible: the composer text', { css: '.one-line-hint { color: transparent !important; }' }],
  ['invisible: the composer hint', { css: '.one-line-hint::placeholder { color: transparent !important; }' }],
  ['covered: the composer', { css: '.one-line-hint { pointer-events: none !important; }' }],
  ['covered: the composer', { css: '.one-line-hint { clip-path: inset(100%) !important; }' }],
  ['covered: a Settings choice', { css: '[role=tablist] { pointer-events: none !important; }' }],
  ['placeholder: the field', { css: '.one-line-hint:placeholder-shown { white-space: normal !important; }' }],
  ['placeholder: the placeholder', { css: '.one-line-hint::placeholder { white-space: normal !important; }' }],
  [' high for a ', { css: '.one-line-hint { height: 1px !important; min-height: 0 !important; max-height: 1px !important; padding: 0 !important; }' }],
  ['placeholder: an empty composer is', { css: '.one-line-hint { min-height: 200px !important; }' }],
  ['composer-offscreen:', { css: '.one-line-hint { position: fixed !important; top: 900px !important; }' }],
  ['fixture: the thread shows no empty project path', { page: dropEmptyIn('.dots.selectable > *') }],
  ['asset-missing:', { page: `fetch('/panel/fonts/not-there.woff2');` }],
  // A tab that does nothing must not pass as a view that measured clean.
  ['not-rendered: accounts never loaded', { page: `addEventListener('click', (e) => { if (e.target.closest?.('button[aria-label="Accounts"]')) e.stopPropagation(); }, true);` }],
  ['no Accounts tab: covered by', { css: 'button[aria-label="Accounts"] { pointer-events: none !important; }' }],
  // Runs the whole wait, so it costs a minute, as does the slow-host fault above.
  ['not-rendered: the panel never settled', { page: `setInterval(() => document.querySelector('.panel')?.append('x'), 10);` }],
  ['no Accounts tab: covered by', { css: '.panel::after { content: ""; position: fixed; inset: 0; z-index: 99999; pointer-events: auto; }' }],
];

async function main() {
  const binary = findChrome();
  if (!binary) {
    console.error('layout-check: no Chrome found; set CHROME_PATH');
    return IN_CI ? 1 : 2;
  }
  const { server, stub } = await serve();
  const origin = `http://127.0.0.1:${server.address().port}/`;
  let chrome = null;
  try {
    chrome = await launch(binary);
    await chrome.cdp('Page.enable');
    await chrome.cdp('Runtime.enable');
    const clean = await walk(chrome, origin, stub);
    for (const f of danglingRefs(OUT)) clean.push(`bundle asset-missing: ${f}`);
    for (const f of clean) console.error(`  ${f}`);
    if (clean.length) {
      console.error(`layout-check: ${clean.length} failed`);
      return 1;
    }
    if (!SELF_TEST) {
      console.log(`layout-check: ok at ${WIDTHS.join(', ')}`);
      return 0;
    }
    let missed = 0;
    for (const [want, fault] of FAULTS) {
      const seen = await walk(chrome, origin, stub, fault, want);
      const caught = seen.some((f) => f.includes(want));
      if (!caught) {
        missed++;
        console.log(`  got: ${seen.join(' / ').slice(0, 300)}`);
      }
      console.log(`${caught ? 'caught' : 'MISSED'}  ${want}`);
    }
    const refs = [
      [{ css: 'url(/panel/fonts/not-there.woff2)' }, 'fonts/not-there.woff2'],
      [{ js: 'import("./chunks/not-there.js")' }, 'chunks/not-there.js'],
      [{ chunk: 'from"./not-there.js"' }, 'chunks/not-there.js'],
    ];
    for (const [extra, name] of refs) {
      if (danglingRefs(OUT, extra).includes(name)) continue;
      missed++;
      console.log(`MISSED  ${name} from ${JSON.stringify(extra)}`);
    }
    const probe = mkdtempSync(join(tmpdir(), 'panel-layout-port-'));
    const portFile = join(probe, 'DevToolsActivePort');
    writeFileSync(portFile, '');
    const early = readPort(portFile);
    writeFileSync(portFile, '61104\n/devtools/browser/x\n');
    const written = readPort(portFile);
    rmSync(probe, { recursive: true, force: true });
    if (early !== null || written !== '61104') {
      missed++;
      console.log(`MISSED  a port file read before Chrome wrote it: ${early}, ${written}`);
    }
    console.log(`layout-check self-test: ${FAULTS.length + refs.length + 1} faults, ${missed} missed`);
    return missed ? 1 : 0;
  } finally {
    await chrome?.close();
    server.closeAllConnections();
    server.close();
  }
}

main().then((code) => process.exit(code), (e) => {
  console.error(`layout-check: ${e.message}`);
  process.exit(1);
});
