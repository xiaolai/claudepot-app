import { signIn } from "@/lib/auth";

/**
 * Three rows, two affordances. OAuth providers (GitHub, Google) are
 * linkable accounts — clicking Connect runs the OAuth flow with that
 * provider; allowDangerousEmailAccountLinking in src/lib/auth.ts attaches
 * it to the current user when the verified email matches. Magic link is
 * not a linkable account in Auth.js's model — the email lives directly
 * on the user row, so this surface only reports it.
 */
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
        <span className="proto-method-label">GitHub</span>
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
        <span className="proto-method-label">Google</span>
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
        <span className="proto-method-label">Magic link · email</span>
        <span className="proto-meta-quiet">{email}</span>
      </li>
    </ul>
  );
}
