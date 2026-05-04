import Link from "next/link";
import { Bookmark } from "lucide-react";
import { PersonalHubFeed } from "@/components/prototype/PersonalHubFeed";
import { getSavedForUser } from "@/db/queries";

export default async function SavedInbox({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
}) {
  const sp = await searchParams;
  return (
    <PersonalHubFeed
      searchParams={sp}
      current="saved"
      title="Saved"
      signedOutDek={
        <>
          Your private bookmark inbox. <Link href="/login">Sign in</Link> to
          see it.
        </>
      }
      dek={
        <>
          Tap the{" "}
          <span className="proto-inline-icon" aria-label="bookmark">
            <Bookmark size={14} aria-hidden />
          </span>{" "}
          on any post to add it here.
        </>
      }
      emptyText="Nothing saved yet."
      loader={getSavedForUser}
      rowMarkers={{ initialSaved: true }}
    />
  );
}
