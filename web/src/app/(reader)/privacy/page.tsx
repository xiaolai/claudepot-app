import type { Metadata } from "next";
import Link from "next/link";

export const metadata: Metadata = {
  title: "Privacy Policy",
  description:
    "What Claudepot collects, what it doesn't, and where your data lives. Both the desktop app and this website are open source.",
};

const LAST_UPDATED = "May 5, 2026";
const CONTACT_EMAIL = "hello@claudepot.com";
const REPO_URL = "https://github.com/xiaolai/claudepot-app";

export default function PrivacyPage() {
  return (
    <div className="proto-page-aside">
      <nav
        className="proto-page-aside-nav proto-page-aside-nav--mobile-hide"
        aria-label="On this page"
      >
        <span className="proto-page-aside-nav-title">On this page</span>
        <ul>
          <li><a href="#summary">The short version</a></li>
          <li><a href="#desktop-app">The desktop app</a></li>
          <li><a href="#website">The website</a></li>
          <li><a href="#oauth-integrations">OAuth integrations</a></li>
          <li><a href="#third-parties">Third-party services</a></li>
          <li><a href="#cookies">Cookies</a></li>
          <li><a href="#your-rights">Your rights</a></li>
          <li><a href="#children">Children</a></li>
          <li><a href="#changes">Changes to this policy</a></li>
          <li><a href="#contact">Contact</a></li>
        </ul>
      </nav>
      <div className="proto-page-aside-content">
        <h1>Privacy Policy</h1>
        <p className="proto-dek">
          Last updated: {LAST_UPDATED}. This policy covers the Claudepot
          desktop app and the website at <code>claudepot.com</code>. Both are
          open source &mdash; you can verify every claim below by reading the
          source at{" "}
          <a href={REPO_URL} target="_blank" rel="noopener noreferrer">
            {REPO_URL.replace("https://", "")}
          </a>
          .
        </p>

        <section id="summary" className="proto-section">
          <h2>The short version</h2>
          <ul>
            <li>
              The desktop app stores everything on your machine. It does not
              send your accounts, transcripts, project data, or telemetry to
              any server we control.
            </li>
            <li>
              The website stores the bare minimum needed for sign-in: your
              email, display name, and avatar URL from your OAuth provider
              (or just your email for magic-link sign-in).
            </li>
            <li>
              We do not sell your data. We do not run advertising. We do not
              use cross-site trackers.
            </li>
            <li>
              You can delete your website account at any time by emailing{" "}
              <a href={`mailto:${CONTACT_EMAIL}`}>{CONTACT_EMAIL}</a>. The
              desktop app has nothing to delete on our side &mdash; it&rsquo;s
              all on your disk.
            </li>
          </ul>
        </section>

        <section id="desktop-app" className="proto-section">
          <h2>The desktop app</h2>
          <p>
            Claudepot is a local-first Tauri desktop app. It reads and writes
            files in your home directory and your operating system&rsquo;s
            keychain. It does <strong>not</strong> have a server. There is no
            account, no telemetry, no analytics beacon, no crash reporter
            phoning home.
          </p>
          <p>What stays on your machine:</p>
          <ul>
            <li>
              <code>~/.claudepot/</code> &mdash; SQLite databases for
              registered accounts, the session index, and a small
              notifications log. Override the location with{" "}
              <code>CLAUDEPOT_DATA_DIR</code>.
            </li>
            <li>
              OS Keychain entries (macOS Keychain, Windows Credential Manager,
              Linux Secret Service) &mdash; for credentials Claudepot manages
              on your behalf.
            </li>
            <li>
              Anything Claude Code or Claude Desktop already writes to your
              disk. Claudepot reads those files; it does not copy them
              elsewhere.
            </li>
          </ul>
          <p>
            If you uninstall the app and remove <code>~/.claudepot/</code>,
            no trace of you remains anywhere.
          </p>
        </section>

        <section id="website" className="proto-section">
          <h2>The website</h2>
          <p>
            <code>claudepot.com</code> is the project&rsquo;s landing site,
            documentation, and reader. Most of it works without an account.
            If you sign in, here is the complete list of what we store:
          </p>
          <ul>
            <li>
              <strong>Email address</strong> &mdash; from your OAuth provider
              or the address you gave to the magic-link login. Used to sign
              you in and to send transactional email (login links, account
              notifications you opted into).
            </li>
            <li>
              <strong>Display name</strong> &mdash; from your OAuth provider,
              if available. Shown next to your activity on the site. You can
              change it in settings.
            </li>
            <li>
              <strong>Avatar URL</strong> &mdash; the picture URL from your
              OAuth provider. We do not host the image; the URL is rendered
              by your browser.
            </li>
            <li>
              <strong>OAuth account linkage</strong> &mdash; the provider name
              (e.g. &ldquo;github&rdquo;) and the provider&rsquo;s opaque
              account ID, so we can recognize you on next sign-in.
            </li>
            <li>
              <strong>Session record</strong> &mdash; a random session token
              and an expiration timestamp, stored in our database and
              referenced by a cookie in your browser.
            </li>
            <li>
              <strong>Anything you submit or post</strong> &mdash; comments,
              votes, saved items, and submissions on the reader side, tied
              to your account.
            </li>
          </ul>
          <p>What we do <em>not</em> store:</p>
          <ul>
            <li>OAuth refresh tokens beyond what Auth.js requires for session continuity. We never use them to read your data on the provider after sign-in.</li>
            <li>Your IP address in application logs. (Vercel may retain edge logs &mdash; see below.)</li>
            <li>Any data from Claude Code or Claude Desktop. The website does not see your Claudepot desktop data.</li>
          </ul>
        </section>

        <section id="oauth-integrations" className="proto-section">
          <h2>OAuth integrations</h2>
          <p>
            Two distinct flows use OAuth, and they are different in how they
            handle your data.
          </p>
          <h3>Sign-in to the website (Google, GitHub, magic-link email)</h3>
          <p>
            When you sign into <code>claudepot.com</code>, we receive your
            email, display name, and avatar URL from the provider. That is
            all. We do not request scopes beyond basic profile + email. We
            never read your repositories, calendar, drive, or messages.
          </p>
          <p>
            Magic-link sign-in via Resend uses your email address only, sent
            once to deliver the link. We do not subscribe you to anything.
          </p>
          <h3>OAuth from the desktop app (Google, GitHub, X, Bluesky)</h3>
          <p>
            When the desktop app integrates with a third-party service, the
            OAuth flow is between you and that provider. The resulting access
            token is stored in your operating system&rsquo;s keychain, on
            your machine. The token never reaches a Claudepot server, because
            the app does not have one. We cannot read it; we cannot revoke
            it remotely. To revoke access, use the provider&rsquo;s account
            settings (e.g. GitHub &rarr; Settings &rarr; Applications) or
            sign out from inside Claudepot.
          </p>
        </section>

        <section id="third-parties" className="proto-section">
          <h2>Third-party services we use for the website</h2>
          <p>
            We pass data to these processors only as far as necessary to
            run the website. Each is bound by their own privacy terms.
          </p>
          <ul>
            <li>
              <strong>Vercel</strong> &mdash; hosts the website and serves
              pages. Vercel may retain edge request logs (IP, user-agent,
              path) for a short period for operational reasons. We use{" "}
              <strong>Vercel Web Analytics</strong> for aggregate page-view
              counts; it does not use cookies and does not track you across
              sites.
            </li>
            <li>
              <strong>Neon</strong> &mdash; hosts our Postgres database
              (the user records described above).
            </li>
            <li>
              <strong>Resend</strong> &mdash; sends transactional email
              (magic-link sign-in, account notifications). Resend processes
              your email address solely to deliver the message.
            </li>
            <li>
              <strong>GitHub, Google</strong> &mdash; OAuth identity
              providers for sign-in. They see that you authenticated to
              Claudepot. We see what they choose to share (email, name,
              avatar).
            </li>
            <li>
              <strong>X (Twitter), Bluesky</strong> &mdash; OAuth identity
              and content providers for desktop-app integrations only. The
              website does not use them.
            </li>
          </ul>
        </section>

        <section id="cookies" className="proto-section">
          <h2>Cookies</h2>
          <p>
            We set one kind of cookie: an HTTP-only, secure session cookie
            that holds your sign-in token. It expires when your session
            ends. We do not set advertising or cross-site tracking cookies.
            Vercel Web Analytics is cookieless.
          </p>
        </section>

        <section id="your-rights" className="proto-section">
          <h2>Your rights</h2>
          <p>
            You can request the following at any time by emailing{" "}
            <a href={`mailto:${CONTACT_EMAIL}`}>{CONTACT_EMAIL}</a> from the
            address tied to your account:
          </p>
          <ul>
            <li>
              <strong>Access</strong> &mdash; a copy of the user record and
              activity tied to your account.
            </li>
            <li>
              <strong>Correction</strong> &mdash; update your display name,
              email, or avatar.
            </li>
            <li>
              <strong>Deletion</strong> &mdash; permanent removal of your
              account and the associated user record. Comments and
              submissions you authored will be either deleted or anonymized,
              your choice.
            </li>
            <li>
              <strong>Portability</strong> &mdash; an export of your account
              data in a machine-readable format.
            </li>
          </ul>
          <p>
            We aim to respond within 30 days. If you are in the EU/UK, you
            also have the right to lodge a complaint with your local
            supervisory authority.
          </p>
        </section>

        <section id="children" className="proto-section">
          <h2>Children</h2>
          <p>
            Claudepot is not directed at children under 13 (or 16 in the
            EU/UK). We do not knowingly create accounts for children. If
            you believe a child has signed up, contact us and we will
            remove the account.
          </p>
        </section>

        <section id="changes" className="proto-section">
          <h2>Changes to this policy</h2>
          <p>
            When we change this page, we update the &ldquo;last
            updated&rdquo; date at the top and commit the change to the
            public repository. The full edit history is in git, so you can
            see exactly what changed and when. Material changes that
            affect existing users will also be announced via email or a
            site banner.
          </p>
        </section>

        <section id="contact" className="proto-section">
          <h2>Contact</h2>
          <p>
            Questions, requests, or concerns:{" "}
            <a href={`mailto:${CONTACT_EMAIL}`}>{CONTACT_EMAIL}</a>. Source
            code and issue tracker:{" "}
            <a href={REPO_URL} target="_blank" rel="noopener noreferrer">
              {REPO_URL.replace("https://", "")}
            </a>
            . See also the{" "}
            <Link href="/terms">Terms of Service</Link>.
          </p>
        </section>
      </div>
    </div>
  );
}
