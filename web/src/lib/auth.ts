/**
 * Auth.js v5 configuration.
 *
 * On first OAuth signup the createUser override calls assignUsername()
 * to derive a clean, unique handle from the provider's display name
 * (or email local-part as fallback) — never a placeholder. The user
 * can then rename themselves inside a grace window via the dashboard;
 * see src/lib/username.ts for the rules and src/app/(reader)/
 * settings/page.tsx for the UI.
 *
 * The session callback exposes `username` from the DB row so server
 * components can build profile URLs without a second hit. Type
 * augmentation lives in src/types/next-auth.d.ts.
 *
 * Until phase 3's full ?as= rip-out, we keep getCurrentUser() as a
 * dev-only simulation alongside auth(). When a real session exists,
 * auth() returns it; otherwise the prototype's ?as= shim is the
 * fallback (see `getViewer` in src/lib/auth.ts).
 */

import NextAuth, { type NextAuthConfig } from "next-auth";
import type { Adapter, AdapterUser } from "next-auth/adapters";
import GitHub from "next-auth/providers/github";
import Google from "next-auth/providers/google";
import Resend from "next-auth/providers/resend";
import { DrizzleAdapter } from "@auth/drizzle-adapter";
import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import {
  users,
  accounts,
  sessions,
  verificationTokens,
} from "@/db/schema";
import { assignUsername } from "@/lib/username-assign";

const GITHUB_ID = process.env.AUTH_GITHUB_ID;
const GITHUB_SECRET = process.env.AUTH_GITHUB_SECRET;
const GOOGLE_ID = process.env.AUTH_GOOGLE_ID;
const GOOGLE_SECRET = process.env.AUTH_GOOGLE_SECRET;
const RESEND_API_KEY = process.env.RESEND_API_KEY;
const EMAIL_FROM = process.env.EMAIL_FROM ?? "ClauDepot <noreply@claudepot.com>";

const providers: NextAuthConfig["providers"] = [];

// Email-based linking: when a user signs in with a second provider
// whose verified email matches an existing account, attach the new
// provider to that account instead of refusing with OAuthAccountNotLinked.
// Safe here because both GitHub and Google verify email server-side
// before issuing the OAuth profile.
if (GITHUB_ID && GITHUB_SECRET) {
  providers.push(
    GitHub({
      clientId: GITHUB_ID,
      clientSecret: GITHUB_SECRET,
      allowDangerousEmailAccountLinking: true,
    }),
  );
}

if (GOOGLE_ID && GOOGLE_SECRET) {
  providers.push(
    Google({
      clientId: GOOGLE_ID,
      clientSecret: GOOGLE_SECRET,
      allowDangerousEmailAccountLinking: true,
    }),
  );
}

if (RESEND_API_KEY) {
  providers.push(Resend({ apiKey: RESEND_API_KEY, from: EMAIL_FROM }));
}

// DrizzleAdapter's generic type is rigid about column types — it expects
// `email` etc. to be PgText, but ours is citext (custom type). The runtime
// behavior is identical (text-compatible). Cast to bypass the type check.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const adapterTables = {
  usersTable: users as any,
  accountsTable: accounts as any,
  sessionsTable: sessions as any,
  verificationTokensTable: verificationTokens as any,
};

const baseAdapter = DrizzleAdapter(db, adapterTables);

// Postgres unique-violation SQLSTATE — what the unique index on
// users.username throws when a concurrent OAuth signup wins the race
// to claim the same candidate handle.
const PG_UNIQUE_VIOLATION = "23505";

function isUniqueViolation(err: unknown): boolean {
  return (
    typeof err === "object" &&
    err !== null &&
    "code" in err &&
    (err as { code?: unknown }).code === PG_UNIQUE_VIOLATION
  );
}

const adapter: Adapter = {
  ...baseAdapter,
  async createUser(data) {
    // assignUsername returns a name that is free at read-time; under
    // concurrent signups two callers may both pick the same candidate
    // and both pass the SELECT. The unique index is the only honest
    // serializer — if the INSERT fails on it, regenerate and retry.
    const MAX_RETRIES = 4;
    for (let attempt = 0; attempt < MAX_RETRIES; attempt += 1) {
      const username = await assignUsername(db, {
        name: data.name,
        email: data.email,
      });
      try {
        const [row] = await db
          .insert(users)
          .values({
            name: data.name ?? null,
            email: data.email,
            emailVerified: data.emailVerified ?? null,
            image: data.image ?? null,
            username,
            avatarUrl: data.image ?? null,
          })
          .returning();
        return row as unknown as AdapterUser;
      } catch (err) {
        if (!isUniqueViolation(err) || attempt === MAX_RETRIES - 1) throw err;
        // Loop: assignUsername will pick a fresh candidate on the next
        // pass, since the conflicting row is now in the table.
      }
    }
    throw new Error("createUser: exhausted retries on unique-violation race");
  },
  async updateUser(data) {
    // Mirror Auth.js `image` into our `avatarUrl` so subsequent OAuth
    // refreshes don't leave the two columns drifting apart. The base
    // adapter only writes `image` on update; without this hook the
    // user's avatar in ClauDepot UI would freeze at the value captured
    // on first signup. `name` and email are passed through unchanged.
    if (!baseAdapter.updateUser) {
      throw new Error("base adapter is missing updateUser");
    }
    const updated = await baseAdapter.updateUser(data);
    if (data.image !== undefined) {
      await db
        .update(users)
        .set({ avatarUrl: data.image ?? null, updatedAt: new Date() })
        .where(eq(users.id, data.id));
    }
    return updated;
  },
};

export const { handlers, signIn, signOut, auth } = NextAuth({
  adapter,
  providers,
  session: { strategy: "database" },
  trustHost: true,
  pages: {
    signIn: "/login",
    verifyRequest: "/login/verify-request",
    error: "/login/error",
  },
  callbacks: {
    // Database-strategy session callback: `user` is the DB row. Expose
    // `username` and `role` so server components can build profile URLs
    // and gate staff-only UI without a second DB hit. Type augmentation
    // lives in src/types/next-auth.d.ts.
    session({ session, user }) {
      const row = user as unknown as {
        username: string;
        role: "user" | "staff" | "locked" | "system";
      };
      session.user.username = row.username;
      session.user.role = row.role;
      return session;
    },
  },
});
