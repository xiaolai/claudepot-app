import { Mail } from "lucide-react";

import { signIn } from "@/lib/auth";

/**
 * Three rows, two affordances. OAuth providers (GitHub, Google) are
 * linkable accounts — clicking Connect runs the OAuth flow with that
 * provider; allowDangerousEmailAccountLinking in src/lib/auth.ts
 * attaches it to the current user when the verified email matches.
 * Magic link is not a linkable account in Auth.js's model — the
 * email lives directly on the user row, so this surface only
 * reports it.
 *
 * Brand-mark exception per .claude/rules/design.md: the GitHub and
 * Google marks are inline SVGs because lucide-react v1+ removed
 * brand icons. Sign-in provider identification is the canonical
 * "secondary chrome / third-party brand identification" use case
 * the rule allows. Both use currentColor so they inherit the theme.
 *
 * The companion CSS in prototype.css (`.proto-method-list > li::before`
 * suppression) prevents the global proto-section bullet from
 * painting on top of these icons; before that suppression the rows
 * showed tiny squashed dots from the prose-list bullet rule.
 */

function GithubMark({ size = 16 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden
    >
      <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.111.82-.261.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
    </svg>
  );
}

function GoogleMark({ size = 16 }: { size?: number }) {
  // Monochrome single-path "G" so it renders consistently with the
  // currentColor pattern used by the GitHub mark above. The Google
  // brand-guidelines polychrome G isn't a fit for paper-mono;
  // monochrome is acceptable for sign-in-method identification per
  // the brand-mark exception.
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden
    >
      <path d="M12.48 10.92v3.28h7.84c-.24 1.84-.853 3.187-1.787 4.133-1.147 1.147-2.933 2.4-6.053 2.4-4.827 0-8.6-3.893-8.6-8.72s3.773-8.72 8.6-8.72c2.6 0 4.507 1.027 5.907 2.347l2.307-2.307C18.747 1.44 16.133 0 12.48 0 5.867 0 .307 5.387.307 12s5.56 12 12.173 12c3.573 0 6.267-1.173 8.373-3.36 2.16-2.16 2.84-5.213 2.84-7.667 0-.76-.053-1.467-.173-2.053H12.48z" />
    </svg>
  );
}

export function SignInMethodsPanel({
  linkedOAuth,
  email,
}: {
  linkedOAuth: string[];
  email: string;
}) {
  const has = (id: string) => linkedOAuth.includes(id);

  return (
    <ul className="proto-method-list">
      <li className="proto-method-row">
        <span className="proto-method-label">
          <GithubMark /> GitHub
        </span>
        {has("github") ? (
          <span className="proto-method-status proto-method-status-on">
            Connected
          </span>
        ) : (
          <form
            action={async () => {
              "use server";
              await signIn("github", {
                redirectTo: "/settings#sign-in-methods",
              });
            }}
          >
            <button type="submit" className="proto-btn-link">
              Connect
            </button>
          </form>
        )}
      </li>

      <li className="proto-method-row">
        <span className="proto-method-label">
          <GoogleMark /> Google
        </span>
        {has("google") ? (
          <span className="proto-method-status proto-method-status-on">
            Connected
          </span>
        ) : (
          <form
            action={async () => {
              "use server";
              await signIn("google", {
                redirectTo: "/settings#sign-in-methods",
              });
            }}
          >
            <button type="submit" className="proto-btn-link">
              Connect
            </button>
          </form>
        )}
      </li>

      <li className="proto-method-row">
        <span className="proto-method-label">
          <Mail size={16} aria-hidden /> Magic link · email
        </span>
        <span className="proto-meta-quiet">{email}</span>
      </li>
    </ul>
  );
}
