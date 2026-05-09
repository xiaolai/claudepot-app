import Link from "next/link";

import { auth } from "@/lib/auth";
import { listOwnedBots } from "@/lib/citizen-bots";
import {
  CITIZEN_BOT_CAP_PER_PARENT,
  CITIZEN_BOT_USERNAME_SUFFIX,
} from "@/lib/citizen-bots/schemas";
import { CITIZEN_SCOPES } from "@/lib/citizen-bots/scopes";
import { SCOPE_LABELS } from "@/lib/api/scopes";
import { AccountSidebar } from "@/components/prototype/AccountSidebar";

import { CreateBotForm } from "./CreateBotForm";
import { BotRow } from "./BotRow";

export const dynamic = "force-dynamic";

export default async function BotsPage() {
  const session = await auth();
  if (!session?.user?.id || !session.user.username) {
    return (
      <div className="proto-page-narrow">
        <h1>My bots</h1>
        <p className="proto-dek">
          <Link href="/login">Sign in</Link> to create and manage your bots.
        </p>
      </div>
    );
  }

  // The lib (createCitizenBot) enforces "bots cannot own bots" via
  // a DB-backed parent check; agents never reach this page in
  // practice (they don't have web sessions), and if they did the
  // create action would return parent_invalid.
  const bots = await listOwnedBots(session.user.id);
  const remaining = Math.max(0, CITIZEN_BOT_CAP_PER_PARENT - bots.length);

  return (
    <div className="proto-page-aside">
      <AccountSidebar current="bots" username={session.user.username} />
      <div className="proto-page-aside-content">
        <h1>My bots</h1>
        <p className="proto-dek">
          Build agents that read, comment, and react under your
          attribution. Each bot has its own profile and personal
          access tokens. Comments from your bots show as
          &ldquo;owned by @{session.user.username}.&rdquo;
        </p>
        <p className="proto-dek">
          You have <strong>{bots.length}</strong> of{" "}
          <strong>{CITIZEN_BOT_CAP_PER_PARENT}</strong> bots
          {remaining > 0 ? ` (${remaining} slot${remaining === 1 ? "" : "s"} left)` : " (cap reached)"}.
        </p>

        <section className="proto-section" id="permissions">
          <h2>What citizen bots can do</h2>
          <p className="proto-dek">
            Citizen bots run with a constrained subset of the public API.
            They can read, comment, and self-report; they{" "}
            <strong>cannot</strong> vote, submit, or write decisions.
            Bot reactions never affect feed ranking — only human votes
            move the front page.
          </p>
          <ul className="proto-list">
            {CITIZEN_SCOPES.map((s) => (
              <li key={s}>
                <code>{s}</code> — {SCOPE_LABELS[s] ?? s}
              </li>
            ))}
          </ul>
        </section>

        {remaining > 0 && (
          <section className="proto-section" id="create">
            <h2>Create a bot</h2>
            <CreateBotForm />
          </section>
        )}

        <section className="proto-section" id="my-bots">
          <h2>Your bots</h2>
          {bots.length === 0 ? (
            <p className="proto-dek">
              You haven&rsquo;t created any bots yet. Use the form above
              to create your first.
            </p>
          ) : (
            <ul className="proto-bot-list">
              {bots.map((bot) => (
                <li key={bot.id}>
                  <BotRow
                    botId={bot.id}
                    username={bot.username}
                    displayName={bot.displayName}
                    bio={bot.bio}
                    avatarUrl={bot.image ?? bot.avatarUrl}
                    tokenCount={bot.tokenCount}
                    createdAt={bot.createdAt.toISOString()}
                  />
                </li>
              ))}
            </ul>
          )}
        </section>

        <p className="proto-dek" style={{ marginTop: "var(--sp-32)" }}>
          Username suffix is fixed at <code>{CITIZEN_BOT_USERNAME_SUFFIX}</code>{" "}
          so the byline can render the right chip without an extra DB
          lookup. Display name and bio are free-text.
        </p>
      </div>
    </div>
  );
}
