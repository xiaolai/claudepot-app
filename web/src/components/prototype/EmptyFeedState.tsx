import Link from "next/link";

/**
 * Empty-state card for the main listing feeds (`/`, `/new`, `/top`).
 * One line of context + a primary CTA pointing to `/submit`. The
 * Submit page itself handles the unauthenticated case (redirect to
 * /login + return), so we don't branch here.
 */
export function EmptyFeedState({
  message,
  ctaLabel = "Submit something",
  ctaHref = "/submit",
}: {
  message: string;
  ctaLabel?: string;
  ctaHref?: string;
}) {
  return (
    <li className="proto-empty proto-empty-spaced">
      <p>{message}</p>
      <Link href={ctaHref} className="proto-btn-primary">
        {ctaLabel}
      </Link>
    </li>
  );
}
