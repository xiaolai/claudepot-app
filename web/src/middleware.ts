import type { NextRequest } from "next/server";
import { NextResponse } from "next/server";

/**
 * Build a Content-Security-Policy header for one HTML response.
 *
 * Pattern is the same as lixiaolai.com's middleware: per-request random
 * nonce that Next.js threads into every inline hydration script via the
 * `x-nonce` request header (any server component that needs the nonce
 * reads it back from `headers()` and passes it to `<script nonce={...}>`).
 *
 * `'strict-dynamic'` means "scripts trusted by nonce can load further
 * scripts" — that's what lets Next's chunk-loader fetch `/_next/static/*`
 * without us having to enumerate every chunk path in `script-src`.
 *
 * `'unsafe-eval'` is kept only in development because Turbopack's HMR
 * client uses `new Function`. Production bundles don't need it.
 *
 * Styles still allow `'unsafe-inline'`. Next's critical-CSS flush and
 * any inline `style="…"` attribute would require per-element nonces
 * that React doesn't mint. Risk ceiling is low — no script execution
 * from style sinks in modern browsers.
 *
 * External origins explicitly allowlisted:
 *   img-src    Google + GitHub avatars (OAuth user.image when the user
 *              hasn't uploaded their own), raw.github (markdown-rewritten
 *              images from GitHub-imported posts), and Vercel Blob
 *              (where setAvatar() persists user-uploaded photos AND
 *              where seeded bot avatars live — see src/lib/avatars.ts).
 *              The Vercel Blob host carries a per-project random
 *              subdomain (e.g. iaomvi8nxzu0duzf.public.blob.…), so the
 *              allowlist uses a wildcard to survive a future store
 *              rotation without a redeploy gap.
 *   frame-src  YouTube-nocookie, Spotify embeds, Apple Podcasts embeds
 *              (see src/lib/embed-attrs.ts)
 *
 * If you add a new image origin (a new CDN, a new OAuth provider, a
 * remote markdown source) you MUST update img-src AND mirror the
 * change in tests/csp.test.ts — that test exists specifically to
 * make accidental removal/omission of an avatar host a failed build,
 * because a silent CSP drop blocks every avatar at once (see the
 * 2026-05-18 incident, fixed in the same commit that added this note).
 *
 * Vercel Analytics + Speed Insights load their scripts from same-origin
 * (`/_vercel/insights/script.js`), so `'self'` covers them — no third-
 * party origin needed in script-src or connect-src.
 */
export function buildCsp(nonce: string): string {
  const isDev = process.env.NODE_ENV !== "production";
  const scriptSrc = [
    "'self'",
    `'nonce-${nonce}'`,
    "'strict-dynamic'",
    ...(isDev ? ["'unsafe-eval'"] : []),
  ].join(" ");

  return [
    "default-src 'self'",
    "base-uri 'self'",
    "object-src 'none'",
    "frame-ancestors 'none'",
    "form-action 'self'",
    `script-src ${scriptSrc}`,
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob: https://lh3.googleusercontent.com https://avatars.githubusercontent.com https://raw.githubusercontent.com https://*.public.blob.vercel-storage.com",
    "font-src 'self' data:",
    "connect-src 'self'",
    "media-src 'self'",
    "frame-src 'self' https://www.youtube-nocookie.com https://open.spotify.com https://embed.podcasts.apple.com",
    "worker-src 'self' blob:",
    "manifest-src 'self'",
    "upgrade-insecure-requests",
  ].join("; ");
}

export function middleware(request: NextRequest): NextResponse {
  // 128 random bits, base64-encoded. Uses Web Crypto + btoa (both
  // available on every Next.js middleware runtime, including Edge —
  // `Buffer` is not). The CSP only needs the value to be unguessable
  // per request and to round-trip through HTTP headers; full base64
  // padding is fine, no need for base64url.
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  let raw = "";
  for (const byte of bytes) raw += String.fromCharCode(byte);
  const nonce = btoa(raw);
  const csp = buildCsp(nonce);

  const requestHeaders = new Headers(request.headers);
  requestHeaders.set("x-nonce", nonce);

  const response = NextResponse.next({
    request: { headers: requestHeaders },
  });
  response.headers.set("Content-Security-Policy", csp);
  return response;
}

export const config = {
  matcher: [
    /*
     * Skip paths that don't need CSP processing:
     *  - /api          — route handlers set their own cookies, and
     *                    middleware's NextResponse.next() merges +
     *                    drops Set-Cookie headers on those responses.
     *  - /_next/static — static bundle output
     *  - /_next/image  — image optimizer
     *  - /favicon.ico  — root favicon
     *  - *.svg/png/... — static image files
     */
    "/((?!api|_next/static|_next/image|favicon.ico|.*\\.(?:svg|png|jpg|jpeg|gif|webp|ico)$).*)",
  ],
};
