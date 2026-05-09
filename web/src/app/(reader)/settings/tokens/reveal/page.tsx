import { redirect } from "next/navigation";

import { auth } from "@/lib/auth";
import { peekRevealCookie, deleteRevealCookie } from "@/lib/api/reveal-cookie";
import { CopyButton } from "./CopyButton";

/**
 * One-time reveal page for a freshly-minted PAT.
 *
 * Reads the encrypted reveal cookie set by mintApiTokenFormAction
 * (read-only — Next.js 15 forbids cookie writes from Server
 * Components, see the digest-4117502548 incident on 2026-05-09)
 * and renders the plaintext server-side directly into HTML. The
 * plaintext IS passed as a prop to CopyButton (a client component)
 * for the clipboard write — meaning it does enter the React client
 * heap for this single page render. The improvement over the
 * previous useActionState flow is scope: the value lives only
 * inside this leaf component until the user navigates away (the
 * page is `force-dynamic`, so BFCache won't stash it).
 *
 * Single-use is enforced by:
 *   - the inline `dismissReveal` Server Action below, fired by
 *     the "Done" button, which deletes the cookie before redirecting
 *   - the 120-second cookie TTL as a backstop for users who
 *     navigate away without clicking Done
 *
 * If the cookie is missing or invalid (direct visit, expired window,
 * already-consumed via Done, or the minting user no longer matches
 * the current session), redirect back to /settings/tokens — the
 * user has nothing to do here without a payload.
 */
export const dynamic = "force-dynamic";

export default async function TokenRevealPage() {
  const session = await auth();
  if (!session?.user?.id) {
    redirect("/login?callbackUrl=/settings/tokens/reveal");
  }
  const payload = await peekRevealCookie(session.user.id);
  if (!payload) {
    redirect("/settings/tokens");
  }

  async function dismissReveal() {
    "use server";
    await deleteRevealCookie();
    redirect("/settings/tokens");
  }

  return (
    <div className="proto-page-narrow">
      <h1>Token minted</h1>
      <p className="proto-form-flash proto-form-flash-ok">
        Token <strong>{payload.tokenName}</strong> ({payload.displayPrefix}…)
        minted. Copy it now — it cannot be shown again after you click
        Done.
      </p>
      <code className="proto-token-plaintext" id="token-plaintext">
        {payload.plaintext}
      </code>
      <div className="proto-form-inline">
        <CopyButton plaintext={payload.plaintext} />
        <form action={dismissReveal}>
          <button type="submit" className="proto-btn-secondary">
            Done
          </button>
        </form>
      </div>
      <p className="proto-empty proto-empty-spaced">
        Treat this string like a password. Anyone with it can act as
        you within the granted scopes until you revoke it. The cookie
        that carries the plaintext expires in 2 minutes; clicking Done
        deletes it immediately.
      </p>
    </div>
  );
}
