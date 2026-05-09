"use client";

import { useState, useTransition } from "react";

import { deleteBotAction } from "@/lib/actions/citizen-bots";

import { MintBotTokenForm } from "./MintBotTokenForm";

type Props = {
  botId: string;
  username: string;
  displayName: string | null;
  bio: string | null;
  avatarUrl: string | null;
  tokenCount: number;
  createdAt: string;
};

export function BotRow({
  botId,
  username,
  displayName,
  bio,
  avatarUrl,
  tokenCount,
  createdAt,
}: Props) {
  const [showMint, setShowMint] = useState(false);
  const [pending, startTransition] = useTransition();
  const [error, setError] = useState<string | null>(null);

  function onDelete() {
    if (
      !window.confirm(
        `Delete @${username}? This revokes all its tokens and clears its profile. The username stays reserved.`,
      )
    ) {
      return;
    }
    setError(null);
    startTransition(async () => {
      const result = await deleteBotAction(botId);
      if (!result.ok) {
        setError(`Could not delete: ${result.reason}`);
      }
    });
  }

  return (
    <article className="proto-bot-card">
      <header className="proto-bot-card-head">
        {avatarUrl ? (
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={avatarUrl}
            alt=""
            className="proto-bot-avatar"
            width={48}
            height={48}
          />
        ) : (
          <div className="proto-bot-avatar proto-bot-avatar-placeholder" aria-hidden />
        )}
        <div className="proto-bot-card-meta">
          <strong>@{username}</strong>
          {displayName && displayName !== username && (
            <span className="proto-bot-card-display"> · {displayName}</span>
          )}
          <div className="proto-bot-card-sub">
            {tokenCount} active token{tokenCount === 1 ? "" : "s"} ·
            created {createdAt.slice(0, 10)}
          </div>
        </div>
        <div className="proto-bot-card-actions">
          <button
            type="button"
            className="proto-btn-secondary"
            onClick={() => setShowMint((v) => !v)}
            aria-expanded={showMint}
          >
            {showMint ? "Hide token mint" : "Mint token"}
          </button>
          <button
            type="button"
            className="proto-mod-btn proto-mod-btn-remove"
            onClick={onDelete}
            disabled={pending}
          >
            {pending ? "Deleting…" : "Delete"}
          </button>
        </div>
      </header>
      {bio && <p className="proto-bot-card-bio">{bio}</p>}
      {error && <p className="proto-form-error">{error}</p>}
      {showMint && (
        <div className="proto-bot-card-mint">
          <MintBotTokenForm botId={botId} username={username} />
        </div>
      )}
    </article>
  );
}
