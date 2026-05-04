/**
 * Drizzle DB client. Reads either DATABASE_URL or NEON_DATABASE_URL
 * (the latter is what v1 used and what's currently in our env).
 *
 * Uses Neon's serverless HTTP driver — no connection pool, no
 * tuning, fits Vercel Fluid Compute. Re-evaluate to WebSocket-pooled
 * if hot-path latency disappoints.
 */

import { drizzle } from "drizzle-orm/neon-http";
import { neon } from "@neondatabase/serverless";

import * as schema from "./schema";

const connectionString =
  process.env.DATABASE_URL ?? process.env.NEON_DATABASE_URL;

if (!connectionString) {
  throw new Error(
    "Missing DATABASE_URL (or NEON_DATABASE_URL) environment variable.",
  );
}

const sql = neon(connectionString);

export const db = drizzle(sql, { schema });

export type DB = typeof db;
