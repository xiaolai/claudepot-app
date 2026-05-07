import Link from "next/link";
import { redirect } from "next/navigation";

import { consumeRevealCookie } from "@/lib/api/reveal-cookie";
import { CopyButton } from "./CopyButton";

/**
 * One-time reveal page for a freshly-minted PAT.
 *
 * Reads the encrypted reveal cookie set by mintApiTokenFormAction,
 * deletes it (single-use), and renders the plaintext server-side
 * directly into HTML. The plaintext is NOT a prop on a client
 * component — only the small CopyButton is client-side, and it gets
 * the value via a defaultValue/inputRef pattern that the user can
 * reach but the React tree doesn't retain after copy.
 *
 * If the cookie is missing or invalid (direct visit, expired window,
 * already-consumed), redirect back to /settings/tokens — the user
 * has nothing to do here without a payload.
 */
export const dynamic = "force-dynamic";

export default async function TokenRevealPage() {
  const payload = await consumeRevealCookie();
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
