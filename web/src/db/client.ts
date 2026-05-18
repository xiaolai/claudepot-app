/**
 * Drizzle DB client. Reads either DATABASE_URL or NEON_DATABASE_URL
 * (the latter is what v1 used and what's currently in our env).
 *
 * Uses Neon's WebSocket-backed `Pool` via `drizzle-orm/neon-serverless`.
 * The HTTP driver (`drizzle-orm/neon-http`) cannot run transactions —
 * `db.transaction(...)` throws "No transactions support in neon-http
 * driver" — and the comments / moderation / admin paths all need
 * `FOR SHARE` row locks bundled with their inserts. The Pool reuses
 * connections across invocations within a warm Lambda, so the
 * per-request overhead is one WebSocket handshake on cold start, not
 * per-request. `@neondatabase/serverless` ≥ 1.x ships a built-in
 * WebSocket polyfill that does not require any `ws`/
 * `neonConfig.webSocketConstructor` plumbing in Vercel's Node runtime.
 *
 * Vercel-managed Neon provisions `DATABASE_URL` as the pooled
 * (PgBouncer) URL — that's the right one for runtime. The unpooled
 * URL exists for migrations only.
 *
 * Runtime constraint: this module-scope `Pool` is correct for the
 * default Vercel Node runtime, where invocations within a warm Lambda
 * can reuse the same pool. It is NOT safe to reuse like this in Edge
 * Functions — `@neondatabase/serverless`'s README explicitly says to
 * create and close `Pool`/`Client` inside a single request when
 * running on Edge. No route in `src/app/**` declares
 * `runtime = "edge"`, so this constraint is documentary today; if a
 * route is ever switched to Edge, it must construct its own per-
 * request Pool instead of importing this `db`.
 *
 * ---
 *
 * Build-time tolerance. Next.js's "Collect page data" step imports
 * every route module at build time to compute `dynamic`, revalidate,
 * and runtime config. Routes that pull in this client (notably
 * `/api/auth/[...nextauth]` via the DrizzleAdapter) would otherwise
 * fail to build whenever `DATABASE_URL` is unset — which is the
 * default state of Vercel's Preview environment when secrets are
 * scoped to Production only.
 *
 * The workaround: at build time we hand Pool a stub connection
 * string. The Pool constructor merely parses the URL; it does not
 * open a connection until the first query. So Drizzle's
 * `DrizzleAdapter(db)` brand check (`is(db, PgDatabase)`) passes
 * during introspection, and the build completes. At runtime, an
 * actual request resolves the real connection string via
 * `getRuntimeConnectionString`, and a missing env var throws a
 * meaningful error rather than masquerading as a Neon connect
 * failure. NEXT_PHASE is set by `next build` to
 * `"phase-production-build"`; outside that phase we behave exactly
 * as before.
 */

import { Pool } from "@neondatabase/serverless";
import { drizzle } from "drizzle-orm/neon-serverless";

import * as schema from "./schema";

const BUILD_TIME_STUB_URL = "postgres://build-time-stub@localhost:5432/none";

/** True only during `next build`'s page-data collection pass. */
function isBuildPhase(): boolean {
  return process.env.NEXT_PHASE === "phase-production-build";
}

/**
 * Real connection string. Throws when truly missing at runtime —
 * the throw site fires at first query, which lands inside a request
 * handler whose error response is observable, instead of crashing
 * the cold start.
 */
function getRuntimeConnectionString(): string {
  const v = process.env.DATABASE_URL ?? process.env.NEON_DATABASE_URL;
  if (!v) {
    throw new Error(
      "Missing DATABASE_URL (or NEON_DATABASE_URL) environment variable.",
    );
  }
  return v;
}

const connectionString = (() => {
  const v = process.env.DATABASE_URL ?? process.env.NEON_DATABASE_URL;
  if (v) return v;
  if (isBuildPhase()) return BUILD_TIME_STUB_URL;
  // Outside build, missing env is a real configuration error —
  // throw immediately so the deploy fails loudly rather than the
  // first user request hitting a confusing Neon connect timeout.
  return getRuntimeConnectionString();
})();

const pool = new Pool({ connectionString });

export const db = drizzle(pool, { schema });

export type DB = typeof db;
