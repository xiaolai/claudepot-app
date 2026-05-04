/**
 * Type augmentation for Auth.js v5. Adds `username` and `role` to
 * session.user, populated from the DB row by the session callback
 * in src/lib/auth.ts. Without this, those fields would be `any` (or
 * trigger TS errors) at every read site.
 */

import "next-auth";

export type UserRole = "user" | "staff" | "locked" | "system";

declare module "next-auth" {
  interface Session {
    user: {
      id: string;
      name?: string | null;
      email: string;
      image?: string | null;
      username: string;
      role: UserRole;
    };
  }
}
