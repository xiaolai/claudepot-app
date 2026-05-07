import Link from "next/link";
import { redirect } from "next/navigation";

import { auth } from "@/lib/auth";
import { consumeRevealCookie } from "@/lib/api/reveal-cookie";
import { CopyButton } from "./CopyButton";

/**
 * One-time reveal page for a freshly-minted PAT.
 *
 * Reads the encrypted reveal cookie set by mintApiTokenFormAction,
 * deletes it (single-use), and renders the plaintext server-side
 * directly into HTML. The plaintext IS passed as a prop to
 * CopyButton (a client component) for the clipboard write — meaning
 * it does enter the React client heap for this single page render.
 * The improvement over the previous useActionState flow is scope:
 * the value lives only inside this leaf component until the user
 * navigates away (the page is `force-dynamic`, so BFCache won't
 * stash it). The previous flow held it in the mint-form's React
 * state across re-renders and route changes.
 *
 * If the cookie is missing or invalid (direct visit, expired window,
 * already-consumed, or the minting user no longer matches the
 * current session), redirect back to /settings/tokens — the user
 * has nothing to do here without a payload.
 */
export const dynamic = "force-dynamic";

export default async function TokenRevealPage() {
  const session = await auth();
  // No session → bounce to login before touching the cookie. We
  // intentionally don't consume here so a brief network blip doesn't
  // burn the user's reveal window; once they log back in within the
  // 120s TTL the redeem still works.
  if (!session?.user?.id) {
    redirect("/login?callbackUrl=/settings/tokens/reveal");
  }
  const payload = await consumeRevealCookie(session.user.id);
  if (!payload) {
    redirect("/settings/tokens");
  }

  return (
    <div className="proto-page-narrow">
      <h1>Token minted</h1>
      <p className="proto-form-flash proto-form-flash-ok">
        Token <strong>{payload.tokenName}</strong> ({payload.displayPrefix}…)
        minted. Copy it now — it cannot be shown again.
      </p>
      <code className="proto-token-plaintext" id="token-plaintext">
        {payload.plaintext}
      </code>
      <div className="proto-form-inline">
        <CopyButton plaintext={payload.plaintext} />
        <Link href="/settings/tokens" className="proto-btn-secondary">
          Done
        </Link>
      </div>
      <p className="proto-empty proto-empty-spaced">
        Treat this string like a password. Anyone with it can act as you
        within the granted scopes until you revoke it.
      </p>
    </div>
  );
}
