import type { Metadata } from "next";
import Link from "next/link";

export const metadata: Metadata = {
  title: "Terms of Service",
  description:
    "The terms under which you use the Claudepot desktop app and the website at claudepot.com. Both are open source.",
};

const LAST_UPDATED = "May 5, 2026";
const CONTACT_EMAIL = "hello@claudepot.com";
const REPO_URL = "https://github.com/xiaolai/claudepot-app";
const LICENSE_URL = `${REPO_URL}/blob/main/LICENSE`;

export default function TermsPage() {
  return (
    <div className="proto-page-aside">
      <nav
        className="proto-page-aside-nav proto-page-aside-nav--mobile-hide"
        aria-label="On this page"
      >
        <span className="proto-page-aside-nav-title">On this page</span>
        <ul>
          <li><a href="#summary">The short version</a></li>
          <li><a href="#what-claudepot-is">What Claudepot is</a></li>
          <li><a href="#open-source">Open source &amp; license</a></li>
          <li><a href="#your-account">Your account</a></li>
          <li><a href="#acceptable-use">Acceptable use</a></li>
          <li><a href="#your-content">Your content</a></li>
          <li><a href="#third-parties">Third-party services</a></li>
          <li><a href="#disclaimers">Disclaimers</a></li>
          <li><a href="#liability">Limitation of liability</a></li>
          <li><a href="#termination">Termination</a></li>
          <li><a href="#changes">Changes to these terms</a></li>
          <li><a href="#governing-law">Governing law</a></li>
          <li><a href="#contact">Contact</a></li>
        </ul>
      </nav>
      <div className="proto-page-aside-content">
        <h1>Terms of Service</h1>
        <p className="proto-dek">
          Last updated: {LAST_UPDATED}. By installing the Claudepot desktop
          app or using the website at <code>claudepot.com</code>, you agree
          to these terms. If you do not agree, do not install the app and
          do not use the website.
        </p>

        <section id="summary" className="proto-section">
          <h2>The short version</h2>
          <ul>
            <li>
              Claudepot is open-source software under the ISC license. You
              can read, copy, modify, and redistribute it. The ISC license
              text is the legally binding statement on the software
              itself; these terms cover the website and our hosted
              services.
            </li>
            <li>
              The desktop app runs on your machine. You are in charge of
              the data it touches.
            </li>
            <li>
              The website is provided as-is. We do our best to keep it
              running but make no guarantees of uptime, accuracy, or
              fitness for any particular purpose.
            </li>
            <li>
              Don&rsquo;t abuse the service: don&rsquo;t try to break it,
              don&rsquo;t scrape it aggressively, don&rsquo;t use it to
              harm others.
            </li>
          </ul>
        </section>

        <section id="what-claudepot-is" className="proto-section">
          <h2>What Claudepot is</h2>
          <p>
            Claudepot is a multi-account control panel for Claude Code and
            Claude Desktop, distributed as a Tauri desktop application.
            The website at <code>claudepot.com</code> is the project&rsquo;s
            landing page, documentation, changelog, download host, and a
            companion reader for AI-tooling content.
          </p>
          <p>
            Both surfaces are built and operated by the project&rsquo;s
            maintainer (currently Xiaolai Li). Claudepot is not affiliated
            with, endorsed by, or sponsored by Anthropic.
          </p>
        </section>

        <section id="open-source" className="proto-section">
          <h2>Open source &amp; license</h2>
          <p>
            The Claudepot desktop app and the website source code are
            licensed under the{" "}
            <a href={LICENSE_URL} target="_blank" rel="noopener noreferrer">
              ISC License
            </a>
            . You may use, copy, modify, and redistribute the code under
            those terms.
          </p>
          <p>
            The ISC license governs the <em>software</em>. These Terms of
            Service govern your use of the <em>hosted website</em> and any
            services we operate at <code>claudepot.com</code>. The two
            documents address different things; both apply together when
            you use the hosted site.
          </p>
          <p>
            Source code, issues, and pull requests:{" "}
            <a href={REPO_URL} target="_blank" rel="noopener noreferrer">
              {REPO_URL.replace("https://", "")}
            </a>
            .
          </p>
        </section>

        <section id="your-account" className="proto-section">
          <h2>Your account</h2>
          <p>
            You can use the website without an account. Some features
            (saving, voting, commenting, submitting) require sign-in via
            GitHub, Google, or a magic-link email. By signing in you
            confirm that:
          </p>
          <ul>
            <li>The email and identity you authenticated with are yours.</li>
            <li>You are old enough to enter a legal contract in your jurisdiction (at least 13, or 16 in the EU/UK).</li>
            <li>You will keep your sign-in credentials reasonably secure.</li>
          </ul>
          <p>
            We may suspend or remove an account that violates these terms.
            You may delete your account at any time &mdash; see the{" "}
            <Link href="/privacy">Privacy Policy</Link>.
          </p>
        </section>

        <section id="acceptable-use" className="proto-section">
          <h2>Acceptable use</h2>
          <p>While using the website you agree not to:</p>
          <ul>
            <li>Attempt to break, overwhelm, or probe the service for vulnerabilities outside a coordinated disclosure.</li>
            <li>Scrape the site at a rate that degrades it for others, or circumvent rate limits.</li>
            <li>Submit content that is illegal, defamatory, infringing, harassing, hateful, or designed to deceive.</li>
            <li>Impersonate another person or misrepresent your affiliation.</li>
            <li>Use the service to distribute malware or to phish.</li>
            <li>Reverse-engineer or interfere with the desktop app in ways that harm other users (modifying it for your own use is fine and is explicitly permitted by the ISC license).</li>
          </ul>
          <p>
            Responsible security disclosures are welcome. Please email{" "}
            <a href={`mailto:${CONTACT_EMAIL}`}>{CONTACT_EMAIL}</a> with
            details before publishing.
          </p>
        </section>

        <section id="your-content" className="proto-section">
          <h2>Your content</h2>
          <p>
            You own what you submit to the website (comments, posts,
            saved items, votes). By submitting content you grant us a
            non-exclusive, worldwide, royalty-free license to host,
            display, and distribute it as part of the service, and to
            include it in aggregated views (e.g. a feed page).
          </p>
          <p>
            We may remove content that violates the acceptable-use rules
            above. We do not pre-moderate; removals happen reactively.
          </p>
        </section>

        <section id="third-parties" className="proto-section">
          <h2>Third-party services</h2>
          <p>
            The desktop app can connect to external providers (Anthropic,
            Google, GitHub, X, Bluesky, and others) on your behalf when
            you authorize it. Your use of those services is governed by
            their own terms; we are not a party to that relationship and
            cannot speak for them. The OAuth tokens that authorize those
            connections live on your device, not on a server we control
            &mdash; see the{" "}
            <Link href="/privacy">Privacy Policy</Link> for details.
          </p>
        </section>

        <section id="disclaimers" className="proto-section">
          <h2>Disclaimers</h2>
          <p>
            The Claudepot software and the website are provided{" "}
            <strong>&ldquo;as is&rdquo; and &ldquo;as available&rdquo;</strong>{" "}
            without warranty of any kind, express or implied, including
            but not limited to merchantability, fitness for a particular
            purpose, and non-infringement.
          </p>
          <p>
            Claudepot reads and modifies files in your home directory and
            entries in your operating system&rsquo;s keychain. While we
            test extensively, software has bugs. Keep backups of anything
            you can&rsquo;t afford to lose. We make no guarantee that the
            software will be free of defects or that the website will be
            available without interruption.
          </p>
        </section>

        <section id="liability" className="proto-section">
          <h2>Limitation of liability</h2>
          <p>
            To the maximum extent permitted by law, the project, its
            maintainers, and contributors are not liable for any
            indirect, incidental, special, consequential, or exemplary
            damages arising out of your use of (or inability to use) the
            software or the website &mdash; including loss of data,
            profits, or goodwill &mdash; even if advised of the
            possibility of such damages.
          </p>
          <p>
            In jurisdictions that do not allow limitation of certain
            warranties or liabilities, our liability is limited to the
            smallest amount permitted by law.
          </p>
        </section>

        <section id="termination" className="proto-section">
          <h2>Termination</h2>
          <p>
            You may stop using the service at any time. Uninstall the app
            and delete <code>~/.claudepot/</code> to remove all local
            data; email us to delete your website account.
          </p>
          <p>
            We may suspend or terminate access to the website if you
            violate these terms, if continued service creates a legal or
            security risk, or if we discontinue the service. We will give
            reasonable notice when practical.
          </p>
        </section>

        <section id="changes" className="proto-section">
          <h2>Changes to these terms</h2>
          <p>
            We may update these terms over time. The &ldquo;last
            updated&rdquo; date at the top will reflect the latest
            change, and the full edit history is visible in git. For
            material changes that affect existing users, we will post a
            notice on the site or email signed-in users. Your continued
            use after the change date constitutes acceptance of the
            updated terms.
          </p>
        </section>

        <section id="governing-law" className="proto-section">
          <h2>Governing law</h2>
          <p>
            These terms and any dispute arising from them are governed by
            the laws applicable to the maintainer&rsquo;s place of
            residence, without regard to conflict-of-law principles. If
            you are a consumer, mandatory consumer-protection laws of
            your country still apply where they grant rights you cannot
            waive.
          </p>
        </section>

        <section id="contact" className="proto-section">
          <h2>Contact</h2>
          <p>
            Questions about these terms:{" "}
            <a href={`mailto:${CONTACT_EMAIL}`}>{CONTACT_EMAIL}</a>. See
            also the{" "}
            <Link href="/privacy">Privacy Policy</Link>.
          </p>
        </section>
      </div>
    </div>
  );
}
