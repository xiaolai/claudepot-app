#!/usr/bin/env node
// Layout guard for Global → Config → Env Variables.
//
// WHY THIS EXISTS
// ---------------
// The pane shipped with its editable list rendering at exactly 0px:
// `.envvar-list` is `flex: 1` (= `flex: 1 1 0%`, base size zero) with
// `min-height: 0`, and the 293-name "Documented nowhere" grid sat beside it
// as a sibling whose default `min-height: auto` refuses to shrink. Free space
// went negative, `flex-grow` only distributes *positive* free space, and the
// list never left basis 0. Every row was in the DOM and none was on screen.
//
// The pane's jsdom tests all passed. jsdom has no layout engine, so
// `getByText` finds a row inside a zero-height container happily. No amount
// of jsdom testing can catch this class of bug.
//
// WHY THE REAL APP, NOT A HARNESS
// -------------------------------
// An earlier version of this script rendered a hand-written HTML harness in
// headless Chrome. It was the wrong instrument: the harness mirrored a DOM
// structure *I chose*, written after the bug was already understood, so it
// could only ever confirm what its author already believed. The bug was
// structural — two sections in the wrong parent — and a transcription of the
// structure cannot find a fault in the structure.
//
// This drives the actual running app over the same WebSocket bridge that
// `scripts/capture-screenshots.mjs` uses (plain JSON, no auth, no
// dependency). It measures what the user sees.
//
// The bridge is debug-only (`#[cfg(debug_assertions)]`,
// src-tauri/src/lib.rs), so this needs a dev build running:
//
//     pnpm tauri dev
//     node scripts/check-envvar-layout.mjs
//
// No `screenshot-fixture` needed — that exists so screenshots don't capture
// real data, and this writes nothing to disk.
//
// Usage:  node scripts/check-envvar-layout.mjs [--debug]
// Exit:   0 assertions pass · 1 an assertion failed · 2 could not run
//         (2 is "app not running" — callers should skip, not fail)

const PORT = process.env.MCP_BRIDGE_PORT ?? 9223;
const DEBUG = process.argv.includes("--debug");

let seq = 0;
const pending = new Map();

function connect() {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://127.0.0.1:${PORT}`);
    ws.addEventListener("error", () =>
      reject(
        Object.assign(
          new Error(
            `no MCP bridge on ${PORT}. The bridge is debug-only; start the ` +
              `app first:\n  pnpm tauri dev`,
          ),
          { code: 2 },
        ),
      ),
    );
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
      msg.success === false
        ? p.reject(new Error(msg.error ?? "bridge error"))
        : p.resolve(msg);
    });
  });
}

function send(ws, command, args) {
  const id = `x${++seq}`;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, command, args }));
    setTimeout(() => {
      if (pending.delete(id)) reject(new Error(`${command} timed out`));
    }, 30_000);
  });
}

const js = (ws, script) =>
  send(ws, "execute_js", { script }).then((r) => r.data ?? r.result);

/** Poll until `probe` returns truthy, or give up. */
async function waitFor(ws, probe, what, tries = 40) {
  for (let i = 0; i < tries; i++) {
    if (await js(ws, probe)) return true;
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error(`timed out waiting for ${what}`);
}

async function main() {
  const ws = await connect();

  // Drive the app to the pane through its own persisted-route + event
  // contract rather than by clicking guessed selectors. A CSS-class guess is
  // exactly the kind of transcription this script exists to avoid, and the
  // app restores whatever section was last open, so a click path would also
  // depend on where it happened to start.
  await js(
    ws,
    `(() => {
      localStorage.setItem('claudepot.global.tab', 'config');
      localStorage.setItem('claudepot.subRoute.global', 'node:virtual:env-vars');
      window.dispatchEvent(new CustomEvent('claudepot:navigate-section', {
        detail: { id: 'global' },
      }));
      return true;
    })()`,
  );
  await waitFor(ws, `!!document.querySelector('.envvar-list')`, "env-vars pane");
  // The list loads asynchronously; measuring an empty one proves nothing.
  await waitFor(
    ws,
    `document.querySelectorAll('.envvar-row').length > 0`,
    "documented rows",
  );

  const MEASURE = `(() => {
      const pane = document.querySelector('.envvar-pane');
      const list = document.querySelector('.envvar-list');
      const lr = list.getBoundingClientRect();
      const rows = [...list.querySelectorAll('.envvar-row')];
      const visible = rows.filter(el => {
        const a = el.getBoundingClientRect();
        return a.bottom > lr.top + 1 && a.top < lr.bottom - 1;
      }).length;
      const scrollers = [pane, ...pane.querySelectorAll('*')].filter(el => {
        const o = getComputedStyle(el).overflowY;
        return o === 'auto' || o === 'scroll';
      }).map(el => el.className || el.tagName);
      const buckets = [...document.querySelectorAll('.envvar-bucket')];
      return {
        listHeight: Math.round(lr.height),
        listScrollHeight: list.scrollHeight,
        visibleRows: visible,
        totalRows: rows.length,
        scrollers,
        bucketsInsideList: buckets.every(b => list.contains(b)),
        bucketCount: buckets.length,
      };
    })()`;

  // Measure BOTH disclosure states. Collapsed is the default; expanded is the
  // worst case, and it is the state the original bug was at its ugliest in —
  // 293 names of inflexible content. Checking only the default would have
  // scored the shipped bug at 136px rather than 0px, because a collapsed
  // appendix is small enough to leave the list some space. A guard that only
  // sees the easy state understates the failure it exists to catch.
  const collapsed = await js(ws, MEASURE);
  await js(
    ws,
    `(() => {
      const d = document.querySelector('.envvar-undocumented-disclosure');
      if (d) d.open = true;
      return true;
    })()`,
  );
  await new Promise((r) => setTimeout(r, 250));
  const expanded = await js(ws, MEASURE);
  // Leave the pane as we found it.
  await js(
    ws,
    `(() => {
      const d = document.querySelector('.envvar-undocumented-disclosure');
      if (d) d.open = false;
      return true;
    })()`,
  );

  ws.close();
  if (DEBUG) {
    console.error("collapsed:", JSON.stringify(collapsed, null, 2));
    console.error("expanded:", JSON.stringify(expanded, null, 2));
  }

  const failures = [];
  for (const [state, m] of [["collapsed", collapsed], ["expanded", expanded]]) {
    if (!(m.listHeight > 0)) {
      failures.push(
        `[${state}] .envvar-list has height ${m.listHeight}px — the editable ` +
          `list is not on screen. Its ${m.listScrollHeight}px of content is clipped.`,
      );
    }
    if (!(m.visibleRows > 0)) {
      failures.push(
        `[${state}] 0 of ${m.totalRows} rows intersect the list viewport.`,
      );
    }
  }
  const r = collapsed;
  if (r.scrollers.length !== 1) {
    failures.push(
      `expected exactly 1 scroll container in the pane, found ` +
        `${r.scrollers.length}: ${r.scrollers.join(", ")}. Nested scrollers ` +
        `chain unpredictably; the pane must not scroll, only the list.`,
    );
  }
  if (r.bucketCount > 0 && !r.bucketsInsideList) {
    failures.push(
      `an appendix bucket is outside .envvar-list — inflexible content beside ` +
        `a flex-basis-0 scroller is what pinned the list to 0px.`,
    );
  }

  if (failures.length) {
    console.error("envvar-layout: FAILED");
    for (const f of failures) console.error(`  ✗ ${f}`);
    process.exit(1);
  }
  console.error(
    `envvar-layout: ok — collapsed ${collapsed.listHeight}px ` +
      `(${collapsed.visibleRows}/${collapsed.totalRows} rows), ` +
      `expanded ${expanded.listHeight}px ` +
      `(${expanded.visibleRows}/${expanded.totalRows} rows), ` +
      `1 scroller, ${r.bucketCount} bucket(s) inside it`,
  );
}

main().catch((e) => {
  console.error(`envvar-layout: ${e.message}`);
  process.exit(e.code === 2 ? 2 : 1);
});
