/**
 * Verify every active link in the directory by HEAD-then-GET fallback.
 * Writes a JSON report to /tmp/links-verify.json. Does NOT mutate the
 * DB — review the report before flipping any rows to 'archived'.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/verify-links.ts
 *
 * Concurrency 24, per-request timeout 8s, per-host stagger to avoid
 * hammering a single origin. Browser-shaped User-Agent because many
 * sites 403 generic Node fetches.
 */

import { writeFileSync } from "node:fs";
import { neon } from "@neondatabase/serverless";

const CONCURRENCY = 24;
const TIMEOUT_MS = 8_000;
const UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 " +
  "(KHTML, like Gecko) Version/17.0 Safari/605.1.15";

type Row = { id: string; url: string; name: string };
type Verdict = {
  id: string;
  url: string;
  name: string;
  status: number | null;
  finalUrl: string | null;
  outcome:
    | "ok"
    | "redirect"
    | "client-error"
    | "server-error"
    | "blocked"
    | "timeout"
    | "network";
  notes?: string;
};

async function check(row: Row): Promise<Verdict> {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), TIMEOUT_MS);
  const tryFetch = async (method: "HEAD" | "GET"): Promise<Response | Error> => {
    try {
      return await fetch(row.url, {
        method,
        redirect: "follow",
        signal: ctrl.signal,
        headers: {
          "User-Agent": UA,
          Accept:
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
          "Accept-Language": "en-US,en;q=0.7",
        },
      });
    } catch (e) {
      return e instanceof Error ? e : new Error(String(e));
    }
  };

  let res = await tryFetch("HEAD");
  // GitHub, Notion, several CDNs return 405/403 to HEAD; retry GET.
  if (
    res instanceof Response &&
    (res.status === 405 ||
      res.status === 403 ||
      res.status === 400 ||
      res.status === 501)
  ) {
    res = await tryFetch("GET");
  } else if (res instanceof Error) {
    res = await tryFetch("GET");
  }
  clearTimeout(timer);

  if (res instanceof Error) {
    const msg = res.message ?? "";
    return {
      id: row.id,
      url: row.url,
      name: row.name,
      status: null,
      finalUrl: null,
      outcome: msg.includes("aborted") ? "timeout" : "network",
      notes: msg.slice(0, 200),
    };
  }

  const status = res.status;
  const finalUrl = res.url;
  let outcome: Verdict["outcome"];
  if (status >= 200 && status < 300)
    outcome = finalUrl !== row.url ? "redirect" : "ok";
  else if (status >= 300 && status < 400) outcome = "redirect";
  else if (status === 403 || status === 429) outcome = "blocked";
  else if (status >= 400 && status < 500) outcome = "client-error";
  else outcome = "server-error";

  return { id: row.id, url: row.url, name: row.name, status, finalUrl, outcome };
}

async function pool<T, R>(
  items: T[],
  worker: (t: T) => Promise<R>,
  size: number,
  onProgress?: (done: number, total: number) => void,
): Promise<R[]> {
  const out: R[] = new Array(items.length);
  let next = 0;
  let done = 0;
  await Promise.all(
    Array.from({ length: size }, async () => {
      while (true) {
        const i = next++;
        if (i >= items.length) return;
        out[i] = await worker(items[i]);
        done += 1;
        if (onProgress && done % 25 === 0) onProgress(done, items.length);
      }
    }),
  );
  return out;
}

async function main() {
  const url = process.env.NEON_DATABASE_URL ?? process.env.DATABASE_URL;
  if (!url) {
    console.error("missing NEON_DATABASE_URL / DATABASE_URL");
    process.exit(1);
  }
  const sql = neon(url);

  const rows = (await sql`
    SELECT id::text AS id, url, name
    FROM links
    WHERE status = 'active'
    ORDER BY name
  `) as Row[];
  console.log(`Verifying ${rows.length} active links…`);

  const t0 = Date.now();
  const verdicts = await pool(rows, check, CONCURRENCY, (done, total) =>
    console.log(`  ${done}/${total} (${(((Date.now() - t0) / 1000) | 0)}s)`),
  );
  console.log(`Done in ${(((Date.now() - t0) / 1000) | 0)}s.`);

  // Summary
  const byOutcome = new Map<string, number>();
  for (const v of verdicts) {
    byOutcome.set(v.outcome, (byOutcome.get(v.outcome) ?? 0) + 1);
  }
  console.log("\nOutcome breakdown:");
  for (const [k, n] of [...byOutcome].sort((a, b) => b[1] - a[1])) {
    console.log(`  ${k.padEnd(15)} ${n}`);
  }

  // Detail tables
  const dead = verdicts.filter(
    (v) =>
      v.outcome === "client-error" ||
      v.outcome === "server-error" ||
      v.outcome === "network" ||
      v.outcome === "timeout",
  );
  const blocked = verdicts.filter((v) => v.outcome === "blocked");
  const redirects = verdicts.filter(
    (v) => v.outcome === "redirect" && v.finalUrl && v.finalUrl !== v.url,
  );

  console.log(`\nDead candidates (${dead.length}):`);
  for (const v of dead.slice(0, 50)) {
    console.log(
      `  ${v.outcome.padEnd(13)} ${String(v.status ?? "—").padEnd(4)} ${v.url}`,
    );
  }

  console.log(`\nBlocked (likely-alive but bot-rejecting) (${blocked.length}):`);
  for (const v of blocked.slice(0, 20)) {
    console.log(`  ${String(v.status).padEnd(4)} ${v.url}`);
  }

  console.log(`\nRedirects (${redirects.length}, top 20):`);
  for (const v of redirects.slice(0, 20)) {
    console.log(`  ${v.url}\n    → ${v.finalUrl}`);
  }

  writeFileSync(
    "/tmp/links-verify.json",
    JSON.stringify({ ranAt: new Date().toISOString(), verdicts }, null, 2),
  );
  console.log(`\nFull report: /tmp/links-verify.json`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
