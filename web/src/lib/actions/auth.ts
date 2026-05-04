"use server";

import { signOut } from "@/lib/auth";

/** Sign-out server action wired into the nav dropdown. NextAuth v5
 *  clears the session cookie and redirects via the response. The
 *  `?as=<username>` dev shim is URL-state, not cookie-state, so it
 *  drops naturally because the redirect target carries no query. */
export async function signOutAction() {
  await signOut({ redirectTo: "/" });
}
