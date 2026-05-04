import Link from "next/link";

import { auth } from "@/lib/auth";
import { listMyApiTokens } from "@/lib/actions/api-tokens";
import { SCOPE_LABELS } from "@/lib/api/scopes";
import { AccountSidebar } from "@/components/prototype/AccountSidebar";
import { MintTokenForm } from "./MintTokenForm";
import { RevokeTokenButton } from "./RevokeTokenButton";

export const dynamic = "force-dynamic";

function formatDate(d: Date | null): string {
  if (!d) return "—";
  return d.toISOString().slice(0, 10);
}

export default async function TokensPage() {
  const session = await auth();
  if (!session?.user?.id || !session.user.username) {
    return (
      <div className="proto-page-narrow">
        <h1>API tokens</h1>
        <p className="proto-dek">
          <Link href="/login">Sign in</Link> to mint and manage API tokens.
        </p>
      </div>
    );
  }

  const tokens = await listMyApiTokens();
  const isStaff =
    session.user.role === "staff" || session.user.role === "system";

  return (
    <div className="proto-page-aside">
      <AccountSidebar current="tokens" username={session.user.username} />
      <div className="proto-page-aside-content">
        <h1>API tokens</h1>
        <p className="proto-dek">
          Personal Access Tokens for the public REST and MCP API. Each token
          identifies you to the API and carries the scopes you assign at
          creation time. Tokens are shown once — copy on creation.
        </p>

        <section id="mint" className="proto-section">
          <h2>Mint a new token</h2>
          <MintTokenForm staff={isStaff} />
        </section>

        <section id="list" className="proto-section">
          <h2>Your tokens</h2>
          {tokens.length === 0 ? (
            <p className="proto-dek">
              You haven&rsquo;t minted any tokens yet.
            </p>
          ) : (
            <ul className="proto-token-list">
              {tokens.map((t) => (
                <li key={t.id} className="proto-token-row">
                  <div className="proto-token-head">
                    <strong className="proto-token-name">{t.name}</strong>
                    <code className="proto-token-prefix">
                      {t.displayPrefix}…
                    </code>
                  </div>
                  <dl className="proto-token-meta">
                    <dt>Scopes</dt>
                    <dd>
                      {t.scopes.length === 0 ? (
                        <em>none</em>
                      ) : (
                        <ul className="proto-token-scopes">
                          {t.scopes.map((s) => (
                            <li key={s}>
                              <code>{s}</code> — {SCOPE_LABELS[s] ?? s}
                            </li>
                          ))}
                        </ul>
                      )}
                    </dd>
                    <dt>Created</dt>
                    <dd>{formatDate(t.createdAt)}</dd>
                    <dt>Last used</dt>
                    <dd>{formatDate(t.lastUsedAt)}</dd>
                    <dt>Expires</dt>
                    <dd>{t.expiresAt ? formatDate(t.expiresAt) : "never"}</dd>
                  </dl>
                  <RevokeTokenButton tokenId={t.id} tokenName={t.name} />
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}
