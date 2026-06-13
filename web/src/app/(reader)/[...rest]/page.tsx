import { notFound } from "next/navigation";

/**
 * Catch-all 404 funnel.
 *
 * The root layout (and the only not-found.tsx boundary) live inside the
 * (reader) route group, and Next only routes *unmatched* URLs to a
 * root-level app/not-found.tsx — a not-found file inside a group
 * catches notFound() throws, not arbitrary garbage paths. So without
 * this segment, /no-such-page rendered Next's unbranded default 404
 * while /post/<bad-id> (which calls notFound()) rendered the branded
 * one.
 *
 * This optional-matched-last catch-all turns every otherwise-unmatched
 * URL into a notFound() throw inside the group, so the branded
 * (reader)/not-found.tsx handles both cases with a real 404 status.
 * Static and dynamic siblings (/post/[id], /u/[username], …) always
 * win over a catch-all, so nothing existing is shadowed.
 */
export default function CatchAllNotFound() {
  notFound();
}
