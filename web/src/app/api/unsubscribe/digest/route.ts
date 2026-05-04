/**
 * One-click unsubscribe endpoint for the weekly digest.
 *
 * Three response shapes:
 *
 *   GET  — Validate the token and render a confirmation page with a
 *          single-button POST form. NO database write happens on GET.
 *          This is the prefetch defense: many mail clients (and
 *          link-scanning bots) issue HEAD/GET on every link before the
 *          recipient acts. If GET unsubscribed immediately, those
 *          prefetches would silently disable the digest.
 *   POST — Either Gmail's RFC 8058 one-click flow (carries
 *          `List-Unsubscribe=One-Click` in the body) OR the
 *          confirmation-form submit. Both verify the token in the URL
 *          and flip digest_weekly to false. Idempotent.
 *
 * The token in the URL IS the credential — HMAC-SHA256(AUTH_SECRET,
 * "digest:" + userId). Forging requires AUTH_SECRET; rotating it
 * invalidates every outstanding link.
 */

import { eq } from "drizzle-orm";
import { NextResponse } from "next/server";

import { db } from "@/db/client";
import { userEmailPrefs } from "@/db/schema";
import { verifyUnsubscribeToken } from "@/lib/email/unsubscribe";

async function disableDigest(userId: string) {
  await db
    .insert(userEmailPrefs)
    .values({ userId, digestWeekly: false, notifyReplies: true })
    .onConflictDoUpdate({
      target: userEmailPrefs.userId,
      set: { digestWeekly: false, updatedAt: new Date() },
    });
}

function badToken(): NextResponse {
  return new NextResponse("Invalid or expired unsubscribe link.", {
    status: 400,
    headers: { "content-type": "text/plain; charset=utf-8" },
  });
}

function escapeHtmlAttr(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export async function GET(req: Request): Promise<Response> {
  const url = new URL(req.url);
  const userId = url.searchParams.get("u");
  const token = url.searchParams.get("t");
  if (!userId || !token) return badToken();
  // Validate the token but DO NOT mutate. A prefetch / link-scanner
  // GET should never disable the digest; only an explicit POST does.
  if (!verifyUnsubscribeToken(userId, token)) return badToken();

  const safeU = escapeHtmlAttr(userId);
  const safeT = escapeHtmlAttr(token);

  const html = `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <title>Unsubscribe · ClauDepot</title>
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta name="robots" content="noindex,nofollow" />
    <style>
      body { font-family: ui-monospace, "JetBrains Mono", monospace; max-width: 36rem; margin: 4rem auto; padding: 0 1.25rem; color: #1f2937; }
      h1 { font-size: 1.25rem; margin: 0 0 1rem; }
      p { line-height: 1.6; }
      button { font: inherit; padding: 0.5rem 1rem; border: 2px solid #1f2937; background: #fff; cursor: pointer; }
      button:hover { background: #1f2937; color: #fff; }
      a { color: inherit; }
    </style>
  </head>
  <body>
    <h1>Unsubscribe from the weekly digest?</h1>
    <p>This will stop the weekly digest email. Reply notifications and other transactional mail are unaffected.</p>
    <form method="post" action="/api/unsubscribe/digest?u=${safeU}&amp;t=${safeT}">
      <input type="hidden" name="confirm" value="1" />
      <button type="submit">Yes, unsubscribe</button>
    </form>
    <p style="margin-top: 2rem;"><a href="/settings">Cancel — back to email preferences</a></p>
  </body>
</html>`;
  return new NextResponse(html, {
    status: 200,
    headers: {
      "content-type": "text/html; charset=utf-8",
      // Belt and suspenders: tell intermediaries not to cache the page,
      // because a cached confirmation form against a rotated AUTH_SECRET
      // would be confusing.
      "cache-control": "no-store, max-age=0",
      "x-robots-tag": "noindex,nofollow",
    },
  });
}

export async function POST(req: Request): Promise<Response> {
  // RFC 8058: the URL carries u + t as query params. The body for one-
  // click is `List-Unsubscribe=One-Click`; for our confirmation form
  // it's `confirm=1`. We accept both and authenticate the action solely
  // via the URL token.
  const url = new URL(req.url);
  const userId = url.searchParams.get("u");
  const token = url.searchParams.get("t");
  if (!userId || !token) return badToken();
  if (!verifyUnsubscribeToken(userId, token)) return badToken();

  await disableDigest(userId);

  // Render a small confirmation when the request looks like a browser
  // form submit; reply with plain text for the RFC 8058 bot path.
  const accept = req.headers.get("accept") ?? "";
  if (accept.includes("text/html")) {
    const html = `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <title>Unsubscribed · ClauDepot</title>
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta name="robots" content="noindex,nofollow" />
    <style>
      body { font-family: ui-monospace, "JetBrains Mono", monospace; max-width: 36rem; margin: 4rem auto; padding: 0 1.25rem; color: #1f2937; }
      h1 { font-size: 1.25rem; margin: 0 0 1rem; }
      p { line-height: 1.6; }
      a { color: inherit; }
    </style>
  </head>
  <body>
    <h1>You&rsquo;re unsubscribed from the weekly digest.</h1>
    <p>You won&rsquo;t receive ClauDepot&rsquo;s weekly digest email anymore. Reply notifications and other transactional mail are unaffected.</p>
    <p>Changed your mind? <a href="/settings">Email preferences</a>.</p>
  </body>
</html>`;
    return new NextResponse(html, {
      status: 200,
      headers: {
        "content-type": "text/html; charset=utf-8",
        "cache-control": "no-store, max-age=0",
      },
    });
  }
  return new NextResponse("Unsubscribed.", {
    status: 200,
    headers: { "content-type": "text/plain; charset=utf-8" },
  });
}
