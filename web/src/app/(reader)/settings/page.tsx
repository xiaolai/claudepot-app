import Link from "next/link";
import { eq } from "drizzle-orm";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { accounts, userEmailPrefs, users } from "@/db/schema";
import {
  requestAccountDeletion,
  requestDataExport,
  updateEmailPrefs,
} from "@/lib/actions/settings";
import { canSelfRename } from "@/lib/username";
import { UsernamePanel } from "./UsernamePanel";
import { SignInMethodsPanel } from "./SignInMethodsPanel";
import { AccountSidebar } from "@/components/prototype/AccountSidebar";

async function loadPrefs(userId: string) {
  const [row] = await db
    .select()
    .from(userEmailPrefs)
    .where(eq(userEmailPrefs.userId, userId))
    .limit(1);
  return row ?? { digestWeekly: true, notifyReplies: true };
}

async function loadUser(userId: string) {
  const [row] = await db
    .select({
      id: users.id,
      username: users.username,
      email: users.email,
      createdAt: users.createdAt,
      usernameLastChangedAt: users.usernameLastChangedAt,
      selfUsernameRenameCount: users.selfUsernameRenameCount,
    })
    .from(users)
    .where(eq(users.id, userId))
    .limit(1);
  return row ?? null;
}

async function loadLinkedProviders(userId: string): Promise<string[]> {
  const rows = await db
    .select({ provider: accounts.provider })
    .from(accounts)
    .where(eq(accounts.userId, userId));
  return rows.map((r) => r.provider);
}

export default async function SettingsPage() {
  const session = await auth();
  if (!session?.user?.id) {
    return (
      <div className="proto-page-narrow">
        <h1>Settings</h1>
        <p className="proto-dek">
          <Link href="/login">Sign in</Link> to manage your preferences.
        </p>
      </div>
    );
  }

  const userId = session.user.id;
  const [prefs, user, linkedProviders] = await Promise.all([
    loadPrefs(userId),
    loadUser(userId),
    loadLinkedProviders(userId),
  ]);

  if (!user) {
    // Session points at a row that no longer exists. Treat as signed-out.
    return (
      <div className="proto-page-narrow">
        <h1>Settings</h1>
        <p className="proto-dek">
          Your account record could not be loaded.{" "}
          <Link href="/login">Sign in again</Link>.
        </p>
      </div>
    );
  }

  const renameDecision = canSelfRename({
    createdAt: new Date(user.createdAt),
    selfUsernameRenameCount: user.selfUsernameRenameCount,
    usernameLastChangedAt: user.usernameLastChangedAt
      ? new Date(user.usernameLastChangedAt)
      : null,
  });

  return (
    <div className="proto-page-aside">
      <AccountSidebar current="settings" username={user.username} />
      <div className="proto-page-aside-content">
      <h1>Settings</h1>
      <p className="proto-dek">
        Signed in as <Link href={`/u/${user.username}`}>@{user.username}</Link>{" "}
        · {user.email}
      </p>

      <section id="username" className="proto-section">
        <h2>Username</h2>
        <UsernamePanel
          currentUsername={user.username}
          decision={renameDecision}
          renamesUsed={user.selfUsernameRenameCount}
        />
      </section>

      <section id="sign-in-methods" className="proto-section">
        <h2>Sign-in methods</h2>
        <SignInMethodsPanel
          linkedOAuth={linkedProviders}
          email={user.email}
        />
      </section>

      <section id="email-preferences" className="proto-section">
        <h2>Email preferences</h2>
        <form
          className="proto-form"
          action={async (formData) => {
            "use server";
            await updateEmailPrefs({
              digestWeekly: formData.get("digestWeekly") === "on",
              notifyReplies: formData.get("notifyReplies") === "on",
            });
          }}
        >
          <label>
            <input
              type="checkbox"
              name="digestWeekly"
              defaultChecked={prefs.digestWeekly}
            />
            Weekly digest (Sunday 12:00 UTC — top posts of the week)
          </label>
          <label>
            <input
              type="checkbox"
              name="notifyReplies"
              defaultChecked={prefs.notifyReplies}
            />
            Email me on replies (only when I haven&rsquo;t been online for 24h)
          </label>
          <button type="submit" className="proto-btn-primary">
            Save preferences
          </button>
        </form>
      </section>

      <section id="privacy" className="proto-section">
        <h2>Privacy</h2>

        <h3>Export my data</h3>
        <p>
          Generate a JSON dump of your submissions, comments, votes, saves,
          and profile. Sent to your email; usually arrives within 10 minutes.
        </p>
        <form
          action={async () => {
            "use server";
            await requestDataExport();
          }}
        >
          <button type="submit" className="proto-btn-primary">
            Request export
          </button>
        </form>

        <h3>Delete my account</h3>
        <p>
          Anonymizes your profile (username, email, bio cleared). Your
          submissions and comments stay in threads, attributed to a deleted
          user. This cannot be undone. Type <code>delete my account</code>{" "}
          below to confirm.
        </p>
        <form
          className="proto-form"
          action={async (formData) => {
            "use server";
            await requestAccountDeletion({
              confirmation: String(formData.get("confirmation") ?? ""),
            });
          }}
        >
          <input
            type="text"
            name="confirmation"
            placeholder="delete my account"
            required
            className="proto-input proto-input-wide"
          />
          <button type="submit" className="proto-mod-btn proto-mod-btn-remove">
            Delete account
          </button>
        </form>
      </section>
      </div>
    </div>
  );
}
