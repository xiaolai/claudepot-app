import Link from "next/link";
import { CircleDashed } from "lucide-react";
import { PersonalHubFeed } from "@/components/prototype/PersonalHubFeed";
import { getPendingForUser } from "@/db/queries";

export default async function PendingInbox({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
}) {
  const sp = await searchParams;
  return (
    <PersonalHubFeed
      searchParams={sp}
      current="pending"
      title="Pending"
      signedOutDek={
        <>
          Submissions of yours awaiting AI review or rejected.{" "}
          <Link href="/login">Sign in</Link> to see them.
        </>
      }
      dek={
        <>Submissions awaiting AI review or rejected. Approved posts move out of this list.</>
      }
      emptyText={
        <>
          <span className="proto-inline-icon" aria-hidden>
            <CircleDashed size={14} />
          </span>{" "}
          Nothing pending or rejected. AI cleared everything.
        </>
      }
      loader={getPendingForUser}
    />
  );
}
