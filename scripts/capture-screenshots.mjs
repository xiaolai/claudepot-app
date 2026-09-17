#!/usr/bin/env node
// Capture the documentation screenshots from a running dev build.
//
// Usage:
//   cargo xtask screenshot-fixture
//   HOME=fixtures/screenshot-profile pnpm tauri dev     # in another shell
//   node scripts/capture-screenshots.mjs
//
// # Why Node and not xtask
//
// The app's MCP bridge speaks WebSocket. Doing that from `xtask` means
// adding tokio + tokio-tungstenite + base64 as direct dependencies, for
// a script that runs by hand a few times a release. Node 22+ ships a
// global `WebSocket`, so this needs **no dependency at all** — and the
// repo already requires Node for vite.
//
// # The protocol
//
// Read off `tauri-plugin-mcp-bridge` 0.12's `websocket.rs`. Plain JSON
// frames, no handshake, no auth:
//
//   ->  { id, command, args }
//   <-  { id, success, data | error }
//
// Commands used here: resize_window, execute_js,
// capture_native_screenshot. `capture_native_screenshot` returns base64
// rather than writing a file, so the writing happens on this side.
//
// # What it does NOT do
//
// It does not check that the app is running against the fixture. Point
// it at a dev build holding real data and it will faithfully capture
// real data. `cargo xtask verify-docs` catches staleness, not leakage —
// the guard against leakage is launching with HOME set, which is the
// operator's job and is stated in the fixture's own output.

import { writeFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";

const PORT = process.env.MCP_BRIDGE_PORT ?? 9223;
const ROOT = process.cwd();

// Logical size; the 2x Retina backing store yields the 2560x1600 the
// existing screenshots use. Changing this desynchronises the set.
const WIDTH = 1280;
const HEIGHT = 800;

const DESTS = ["assets/screenshots", "web/public/screenshots"];

/**
 * One row per screenshot. `nav` is the sidebar label; `tab` is an
 * optional sub-tab inside the section; `settle` is text that must be
 * on screen before capturing, which is what makes the run
 * deterministic rather than a race against React.
 */
const SHOTS = [
  { file: "accounts.png", nav: "Accounts", settle: "ACCOUNTS" },
  // Every section with tabs pins one, and settles on text only that tab
  // renders. Activities remembers its last tab across launches
  // (`claudepot.events.tab`), and this row used to settle on "Mark all
  // seen" — a header button on every tab — so a run after someone had
  // looked at Cost captured Cost. The loop below now refuses an
  // unpinned row in a tabbed view, so the same slip cannot recur.
  { file: "activities.png", nav: "Activities", tab: "Stream", settle: "Severity" },
  { file: "projects.png", nav: "Projects", tab: "All", settle: "Select a project" },
  { file: "memory.png", nav: "Knowledge", tab: "Dashboard", settle: "Across all projects" },
  { file: "keys.png", nav: "Keys", settle: "KEYS" },
  { file: "third-parties.png", nav: "Providers", settle: "PROVIDERS" },
  { file: "automations.png", nav: "Agents", settle: "AGENTS" },
  // The section is labelled "Config" in the sidebar and its first
  // sub-tab "Files" (`shell:sections.config`, `global:tabs.config`);
  // this row said "Global" / "Config" from before both were renamed
  // and never settled again, so the shot silently stayed at its
  // 2026-08-15 capture. Under the fixture the Files tab opens on the
  // Env variables pane, whose body reads "N of N documented variables";
  // settle on that CONTENT (count-free), never on a tab BUTTON label —
  // a button is present whichever tab is active, so the weaker string
  // matched instantly and captured whatever sub-tab the app happened to
  // remember, the exact race settling exists to stop. "Config home" is
  // the preview shown only when the config-dir node is selected.
  { file: "global.png", nav: "Config", tab: "Files", settle: "documented variables" },
  { file: "settings.png", nav: "Settings", tab: "Retention", settle: "TRANSCRIPT RETENTION" },
];

let seq = 0;
const pending = new Map();

function connect() {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://127.0.0.1:${PORT}`);
    const fail = () =>
      reject(
        new Error(
          `no MCP bridge on ${PORT}. Start the app first:\n` +
            `  cargo xtask screenshot-fixture\n` +
            `  HOME=$PWD/fixtures/screenshot-profile pnpm tauri dev`,
        ),
      );
    ws.addEventListener("error", fail);
    ws.addEventListener("open", () => resolve(ws));
    ws.addEventListener("message", (ev) => {
      let msg;
      try {
        msg = JSON.parse(ev.data);
      } catch {
        return;
      }
      const p = pending.get(msg.id);
      if (!p) return;
      pending.delete(msg.id);
      msg.success === false ? p.reject(new Error(msg.error ?? "bridge error")) : p.resolve(msg);
    });
  });
}

function send(ws, command, args) {
  const id = `x${++seq}`;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, command, args }));
    // A hung request must not hang the whole run.
    setTimeout(() => {
      if (pending.delete(id)) reject(new Error(`${command} timed out`));
    }, 30_000);
  });
}

const js = (ws, script) => send(ws, "execute_js", { script }).then((r) => r.data ?? r.result);

/** A label with its count badge removed: "Accounts4", "All · 8". */
const BARE = String.raw`(s) => (s || '').trim().replace(/[\s·]*\(?\d+\)?$/, '').trim()`;

/** Click a sidebar entry, then the tab if one is named. Returns what was
 *  found, so a renamed label is a failure rather than a silent no-op —
 *  the settle text alone cannot tell "wrong tab" from "right tab". */
async function navigate(ws, label, tab) {
  const nav = await js(
    ws,
    `(() => {
      const bare = ${BARE};
      const aside = document.querySelector('aside') || document;
      const el = [...aside.querySelectorAll('button,a,[role="button"]')]
        .find(e => bare(e.textContent) === ${JSON.stringify(label)});
      if (el) el.click();
      return !!el;
    })()`,
  );
  if (nav !== true || !tab) return { nav: nav === true, tab: null };
  await sleep(400);
  const found = await js(
    ws,
    `(() => {
      const bare = ${BARE};
      const t = [...document.querySelectorAll('button,[role="tab"]')]
        .find(e => bare(e.textContent) === ${JSON.stringify(tab)});
      if (t) t.click();
      return !!t;
    })()`,
  );
  return { nav: true, tab: found === true };
}

/** After settling: does the view have tabs the row did not pin, and is a
 *  pinned `role="tab"` actually the selected one? "ok" or the reason. */
function tabState(ws, tab) {
  return js(
    ws,
    `(() => {
      const bare = ${BARE};
      const tabs = [...document.querySelectorAll('[role="tab"]')];
      const want = ${JSON.stringify(tab ?? null)};
      if (!want) {
        return tabs.length === 0
          ? 'ok'
          : 'this view has tabs (' + tabs.map(t => bare(t.textContent)).join(', ') + ') — pin one with tab:';
      }
      const t = tabs.find(e => bare(e.textContent) === want);
      if (!t) return 'ok'; // a pane button, not a role="tab" — nothing to read
      return t.getAttribute('aria-selected') === 'true' ? 'ok' : 'tab "' + want + '" is not the selected one';
    })()`,
  );
}

/** Poll until the settle text appears. Beats a fixed sleep: a slow pane
 *  would otherwise be captured mid-render and look broken.
 *
 *  Case-insensitive on purpose. Some headers are uppercase in the DOM
 *  and others are uppercased by CSS, so `innerText` casing varies per
 *  surface — matching exactly meant two panes silently never settled. */
async function waitForText(ws, text, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  const needle = text.toLowerCase();
  while (Date.now() < deadline) {
    const seen = await js(
      ws,
      `document.body.innerText.toLowerCase().includes(${JSON.stringify(needle)})`,
    );
    if (seen === true) return true;
    await sleep(250);
  }
  return false;
}

/** Poll until the sidebar is expanded in the DOM **and** as wide as
 *  `--sidebar-width` resolves to. The attribute alone is React state; the
 *  rendered width is what the image will show, and the two diverged once.
 *  Returns "ok" or a description of what was on screen instead. */
async function waitForExpandedSidebar(ws, timeoutMs = 5_000) {
  const deadline = Date.now() + timeoutMs;
  let last = "never measured";
  while (Date.now() < deadline) {
    last = await js(
      ws,
      `(() => {
        const aside = document.querySelector('aside');
        if (!aside) return 'no-aside';
        if (aside.hasAttribute('data-collapsed')) return 'collapsed';
        const probe = document.createElement('div');
        probe.style.cssText = 'position:absolute;visibility:hidden;width:var(--sidebar-width)';
        document.body.appendChild(probe);
        const want = probe.getBoundingClientRect().width;
        probe.remove();
        const got = aside.getBoundingClientRect().width;
        if (!(want > 0)) return 'no --sidebar-width';
        return Math.abs(got - want) < 1 ? 'ok' : 'width ' + got + 'px, expected ' + want + 'px';
      })()`,
    );
    if (last === "ok") return last;
    await sleep(100);
  }
  return last;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  const ws = await connect();
  await send(ws, "resize_window", { width: WIDTH, height: HEIGHT, logical: true });
  await sleep(500);

  // The sidebar starts collapsed to its rail on a fresh profile, and the
  // fixture home is always fresh. The rail hides the labels `navigate`
  // matches on, and a rail is not what the docs should show — so expand
  // it first, through the same toggle a user would press (the sidebar's
  // own chevron carries `aria-expanded`, so this needs no locale string).
  // A missing sidebar or toggle is a failure, not a skip: every shot
  // below would otherwise navigate nowhere and settle on the wrong pane.
  //
  // Transitions are switched off first. The sidebar animates its width,
  // and a click is not a layout: one run clicked the chevron, reported
  // "expanded", and captured all nine shots with the expanded content
  // squeezed into a rail that had not grown — the width animation had
  // not advanced in a window that had only just opened. A still image
  // has no use for an animation, and without one there is no clock to
  // stall.
  await js(
    ws,
    `(() => {
      if (document.getElementById('capture-no-transitions')) return true;
      const s = document.createElement('style');
      s.id = 'capture-no-transitions';
      s.textContent = '*, *::before, *::after { transition: none !important; }';
      document.head.appendChild(s);
      return true;
    })()`,
  );
  const sidebar = await js(
    ws,
    `(() => {
      const aside = document.querySelector('aside');
      if (!aside) return 'no-aside';
      if (!aside.hasAttribute('data-collapsed')) return 'already-expanded';
      const toggle = [...aside.querySelectorAll('button')]
        .find(b => b.getAttribute('aria-expanded') === 'false');
      if (!toggle) return 'no-toggle';
      toggle.click();
      return 'expanded';
    })()`,
  );
  if (sidebar !== "expanded" && sidebar !== "already-expanded") {
    throw new Error(`could not expand the sidebar before capturing: ${sidebar}`);
  }
  const layout = await waitForExpandedSidebar(ws);
  if (layout !== "ok") {
    throw new Error(`the sidebar did not lay out expanded: ${layout}`);
  }

  let ok = 0;
  const failures = [];
  for (const shot of SHOTS) {
    const went = await navigate(ws, shot.nav, shot.tab);
    if (!went.nav || went.tab === false) {
      failures.push(`${shot.file}: no ${went.nav ? `tab "${shot.tab}"` : `sidebar entry "${shot.nav}"`} to click — skipped`);
      continue;
    }
    if (!(await waitForText(ws, shot.settle))) {
      failures.push(`${shot.file}: "${shot.settle}" never appeared — skipped, not captured blank`);
      continue;
    }
    await sleep(350); // let late renders land before the pixel grab
    // Checked per shot, not once: the state is what the image shows, and
    // an assertion made before the loop says nothing about shot nine.
    const layout = await waitForExpandedSidebar(ws);
    if (layout !== "ok") {
      failures.push(`${shot.file}: sidebar not expanded at capture (${layout}) — skipped`);
      continue;
    }
    const tabs = await tabState(ws, shot.tab);
    if (tabs !== "ok") {
      failures.push(`${shot.file}: ${tabs} — skipped`);
      continue;
    }
    const res = await send(ws, "capture_native_screenshot", { format: "png" });
    const b64 = res.data?.image ?? res.data?.base64 ?? res.data;
    if (typeof b64 !== "string") {
      failures.push(`${shot.file}: bridge returned no image payload`);
      continue;
    }
    const buf = Buffer.from(b64.replace(/^data:image\/\w+;base64,/, ""), "base64");
    for (const dest of DESTS) {
      const out = join(ROOT, dest, shot.file);
      mkdirSync(dirname(out), { recursive: true });
      writeFileSync(out, buf);
    }
    console.log(`  ${shot.file.padEnd(20)} ${(buf.length / 1024).toFixed(0)} KB`);
    ok++;
  }
  // Leave the app as it was found; it is someone's running dev build.
  await js(ws, `(() => { document.getElementById('capture-no-transitions')?.remove(); return true; })()`);
  ws.close();

  console.log(`\n${ok}/${SHOTS.length} captured into ${DESTS.join(" and ")}`);
  if (failures.length) {
    console.error("\nfailed:");
    for (const f of failures) console.error(`  - ${f}`);
    process.exit(1);
  }
  for (const d of DESTS) if (!existsSync(join(ROOT, d))) console.error(`missing dest: ${d}`);
}

main().catch((e) => {
  console.error(String(e.message ?? e));
  process.exit(1);
});
