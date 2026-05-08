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
 * WebSocket polyfill that works in Vercel's Node runtime and Edge
 * runtime without any `ws`/`neonConfig.webSocketConstructor` plumbing.
 *
 * Vercel-managed Neon provisions `DATABASE_URL` as the pooled
 * (PgBouncer) URL — that's the right one for runtime. The unpooled
 * URL exists for migrations only.
 */

import { Pool } from "@neondatabase/serverless";
import { drizzle } from "drizzle-orm/neon-serverless";

import * as schema from "./schema";

const connectionString =
  process.env.DATABASE_URL ?? process.env.NEON_DATABASE_URL;

if (!connectionString) {
  throw new Error(
    "Missing DATABASE_URL (or NEON_DATABASE_URL) environment variable.",
  );
}

const pool = new Pool({ connectionString });

export const db = drizzle(pool, { schema });

export type DB = typeof db;
