import nextMDX from "@next/mdx";

const withMDX = nextMDX({
  extension: /\.mdx?$/,
});

/** @type {import('next').NextConfig} */
const nextConfig = {
  output: "standalone",
  pageExtensions: ["ts", "tsx", "mdx"],
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
