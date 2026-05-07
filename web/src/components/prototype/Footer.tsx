import Link from "next/link";

const REPO_URL = "https://github.com/xiaolai/claudepot-app";
const YEAR = new Date().getFullYear();

export function Footer() {
  return (
    <footer className="proto-footer" aria-label="Site footer">
      <div className="proto-footer-inner">
        <span className="proto-footer-brand">
          &copy; {YEAR} ClauDepot
        </span>
        <nav className="proto-footer-nav" aria-label="Legal and project links">
          <Link href="/about">About</Link>
          <Link href="/help">Help</Link>
          <Link href="/api">API</Link>
          <Link href="/privacy">Privacy</Link>
          <Link href="/terms">Terms</Link>
          <a href={REPO_URL} target="_blank" rel="noopener noreferrer">
            GitHub
          </a>
        </nav>
      </div>
    </footer>
  );
}
