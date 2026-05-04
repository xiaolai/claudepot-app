import { notFound } from "next/navigation";
import Link from "next/link";
import { SubmissionRow } from "@/components/prototype/SubmissionRow";
import { UserAvatar } from "@/components/prototype/Avatar";
import { getSubmissionsByUser, getUser } from "@/db/queries";

const TABS = ["submissions", "comments"] as const;
type Tab = (typeof TABS)[number];

const TAB_LABELS: Record<Tab, string> = {
  submissions: "Submissions",
  comments: "Comments",
};

export default async function ProfilePage({
  params,
  searchParams,
}: {
  params: Promise<{ username: string }>;
  searchParams: Promise<{ tab?: string; as?: string }>;
}) {
  const { username } = await params;
  const sp = await searchParams;
  const tab: Tab = TABS.includes(sp.tab as Tab) ? (sp.tab as Tab) : "submissions";

  const user = await getUser(username);
  if (!user) notFound();

  const submissions = await getSubmissionsByUser(username);

  const linkSuffix = sp.as ? `&as=${sp.as}` : "";
  const baseSuffix = sp.as ? `?as=${sp.as}` : "";

  return (
    <div className="proto-page">
      <header className="proto-profile-header">
        <UserAvatar
          username={user.username}
          imageUrl={user.image_url}
          size={64}
        />
        <div className="proto-profile-header-text">
          <h1>{user.display_name}</h1>
          <span className="proto-profile-meta">@{user.username}</span>
        </div>
      </header>
      <p className="proto-profile-bio">{user.bio}</p>
      <div className="proto-profile-meta">
        karma {user.karma} · joined {user.joined} · via {user.provider}
      </div>

      <nav className="proto-tabs" aria-label="Profile sections">
        {TABS.map((t) => (
          <Link
            key={t}
            href={
              t === "submissions"
                ? `/u/${username}${baseSuffix}`
                : `/u/${username}?tab=${t}${linkSuffix}`
            }
            aria-current={tab === t ? "page" : undefined}
          >
            {TAB_LABELS[t]}
          </Link>
        ))}
      </nav>

      <ol className="proto-feed">
        {tab === "submissions" &&
          (submissions.length === 0 ? (
            <li className="proto-empty">No submissions yet.</li>
          ) : (
            submissions.map((s) => <SubmissionRow key={s.id} submission={s} />)
          ))}
        {tab === "comments" && (
          <li className="proto-empty">
            Comment history coming in implementation.
          </li>
        )}
      </ol>
    </div>
  );
}
