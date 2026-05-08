import { redirect } from "next/navigation";

/**
 * /new → / (308 permanent). Recent is now the default feed view at
 * the root URL, so /new is preserved as a bookmark redirect rather
 * than a duplicate route. Cursor params are dropped — the root view
 * paginates via its own ?cursor= and a stale /new?cursor= would have
 * been off the head anyway.
 */
export default function NewFeedRedirect(): never {
  redirect("/");
}
