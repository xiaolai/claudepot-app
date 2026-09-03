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
  { file: "activities.png", nav: "Activities", settle: "Mark all seen" },
  { file: "projects.png", nav: "Projects", settle: "PROJECTS" },
  { file: "memory.png", nav: "Knowledge", settle: "KNOWLEDGE" },
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

/** Click a sidebar entry. Nav labels carry a count badge ("Accounts4"), so
 *  match on the leading label rather than the whole string. */
async function navigate(ws, label, tab) {
  await js(
    ws,
    `(() => {
      const aside = document.querySelector('aside') || document;
      const el = [...aside.querySelectorAll('button,a,[role="button"]')]
        .find(e => (e.textContent||'').trim().replace(/\\d+$/,'').trim() === ${JSON.stringify(label)});
      if (el) el.click();
      return !!el;
    })()`,
  );
  if (tab) {
    await sleep(400);
    await js(
      ws,
      `(() => {
        const t = [...document.querySelectorAll('button,[role="tab"]')]
          .find(e => (e.textContent||'').trim() === ${JSON.stringify(tab)});
        if (t) t.click();
        return !!t;
      })()`,
    );
  }
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

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  const ws = await connect();
  await send(ws, "resize_window", { width: WIDTH, height: HEIGHT, logical: true });
  await sleep(500);

  let ok = 0;
  const failures = [];
  for (const shot of SHOTS) {
    await navigate(ws, shot.nav, shot.tab);
    if (!(await waitForText(ws, shot.settle))) {
      failures.push(`${shot.file}: "${shot.settle}" never appeared — skipped, not captured blank`);
      continue;
    }
    await sleep(350); // let transitions finish before the pixel grab
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
