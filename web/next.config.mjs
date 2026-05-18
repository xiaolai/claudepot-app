import nextMDX from "@next/mdx";

const withMDX = nextMDX({
  extension: /\.mdx?$/,
});

/** @type {import('next').NextConfig} */
const nextConfig = {
  output: "standalone",
  pageExtensions: ["ts", "tsx", "mdx"],
  async headers() {
    // Static security headers applied to every response. CSP is NOT set
    // here — it's emitted per-request by `src/middleware.ts` with a nonce
    // that Next.js propagates into its inline hydration scripts. Setting
    // CSP in both places would cause browsers to intersect the two
    // policies and drop the nonce, breaking hydration.
    const securityHeaders = [
      {
        key: "Strict-Transport-Security",
        value: "max-age=63072000; includeSubDomains; preload",
      },
      { key: "X-Content-Type-Options", value: "nosniff" },
      { key: "X-Frame-Options", value: "DENY" },
      { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
      {
        key: "Permissions-Policy",
        // FLoC opt-out (interest-cohort=()) keeps the page out of
        // Google's deprecated ad-cohort scheme. The rest mute APIs
        // we never use, so a compromised script can't reach them.
        value:
          "camera=(), microphone=(), geolocation=(), payment=(), usb=(), interest-cohort=()",
      },
      // Same-origin opener prevents `window.opener` cross-origin reads
      // from popups. Cheap XS-Leaks protection. We do NOT set COEP
      // require-corp — that's only useful for SharedArrayBuffer or
      // multithreaded WASM, neither of which this app uses, and it
      // would block every cross-origin asset without a CORP header.
      { key: "Cross-Origin-Opener-Policy", value: "same-origin" },
    ];
    return [{ source: "/:path*", headers: securityHeaders }];
  },
  async redirects() {
    return [
      // Admin redesign — power tools moved under /admin/console/*.
      // 308 (permanent) so external bookmarks update; query strings
      // (e.g. ?as=) are preserved by Next.js by default.
      {
        source: "/admin/users",
        destination: "/admin/console/users",
        permanent: true,
      },
      {
        source: "/admin/users/:path*",
        destination: "/admin/console/users/:path*",
        permanent: true,
      },
      {
        source: "/admin/flags",
        destination: "/admin/console/vocabulary",
        permanent: true,
      },
      {
        source: "/admin/flags/:path*",
        destination: "/admin/console/vocabulary/:path*",
        permanent: true,
      },
      {
        source: "/admin/policy-prompt",
        destination: "/admin/console/policy",
        permanent: true,
      },
      {
        source: "/admin/policy-prompt/:path*",
        destination: "/admin/console/policy/:path*",
        permanent: true,
      },
    ];
  },
};

export default withMDX(nextConfig);
