import Link from "next/link";
import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Download",
  description:
    "Download ClauDepot for macOS, Linux, or Windows. Signed binaries on every release.",
};

interface ReleaseAsset {
  name: string;
  browser_download_url: string;
  size: number;
}

interface Release {
  tag_name: string;
  name: string;
  body: string;
  published_at: string;
  assets: ReleaseAsset[];
}

const REPO = "xiaolai/claudepot-app";

// Fetch GitHub Releases at build time, then ISR every 5 minutes.
async function fetchLatestRelease(): Promise<Release | null> {
  try {
    const r = await fetch(
      `https://api.github.com/repos/${REPO}/releases/latest`,
      {
        headers: { Accept: "application/vnd.github+json" },
        next: { revalidate: 300 },
      },
    );
    if (!r.ok) return null;
    return (await r.json()) as Release;
  } catch {
    return null;
  }
}

interface PlatformAsset {
  label: string;
  match: RegExp;
  hint: string;
}

const PLATFORMS: PlatformAsset[] = [
  { label: "macOS · Apple Silicon", match: /aarch64.*\.dmg$/i, hint: ".dmg" },
  { label: "macOS · Intel", match: /x86_64.*\.dmg$/i, hint: ".dmg" },
  { label: "Linux · ARM64 (tarball)", match: /aarch64-linux\.tar\.gz$/i, hint: ".tar.gz" },
  { label: "Linux · x86_64 (tarball)", match: /x86_64-linux\.tar\.gz$/i, hint: ".tar.gz" },
  { label: "Linux · x86_64 (.deb)", match: /x86_64\.deb$/i, hint: ".deb" },
  { label: "Linux · x86_64 (AppImage)", match: /x86_64\.AppImage$/i, hint: ".AppImage" },
  { label: "Windows · x86_64 (.msi)", match: /x86_64.*\.msi$/i, hint: ".msi" },
  { label: "Windows · x86_64 (setup.exe)", match: /x86_64-setup\.exe$/i, hint: ".exe" },
];

function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

export default async function DownloadPage() {
  const release = await fetchLatestRelease();

  return (
    <article>
      <h1>Download</h1>

      {release ? (
        <p>
          Latest: <strong>{release.tag_name}</strong>, published{" "}
          {new Date(release.published_at).toISOString().slice(0, 10)}.
        </p>
      ) : (
        <p>
          Couldn&rsquo;t reach the GitHub Releases API right now.
          You can browse releases directly at{" "}
          <Link href={`https://github.com/${REPO}/releases`}>
            github.com/{REPO}/releases
          </Link>
          .
        </p>
      )}

      <h2>By platform</h2>
      <ul>
        {PLATFORMS.map((p) => {
          const asset = release?.assets.find((a) => p.match.test(a.name));
          return (
            <li key={p.label}>
              <strong>{p.label}</strong> &mdash;{" "}
              {asset ? (
                <Link href={asset.browser_download_url}>
                  {asset.name} ({bytes(asset.size)})
                </Link>
              ) : (
                <em>asset not present in latest release ({p.hint})</em>
              )}
            </li>
          );
        })}
      </ul>

      <h2>Package managers</h2>
      <p>
        On macOS or Linux:
      </p>
      <pre><code>{`brew install --cask xiaolai/tap/claudepot`}</code></pre>
      <p>
        Updates via <code>brew upgrade</code>.
      </p>

      <h2>Verify</h2>
      <p>
        macOS releases are signed and notarized. Windows releases are signed
        with a code-signing certificate. Linux releases are SHA256-summed in
        the release notes.
      </p>

      <p>
        Source on{" "}
        <Link href={`https://github.com/${REPO}`}>GitHub</Link>. Build from
        source: see <Link href="/app/install">Install</Link>.
      </p>
    </article>
  );
}
