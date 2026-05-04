import Link from "next/link";
import { AlertCircle } from "lucide-react";

type AuthErrorCode =
  | "Configuration"
  | "AccessDenied"
  | "Verification"
  | "OAuthAccountNotLinked"
  | "Default";

const MESSAGES: Record<AuthErrorCode, { title: string; body: string }> = {
  Configuration: {
    title: "Server configuration",
    body: "The sign-in service is misconfigured. The administrator has been notified.",
  },
  AccessDenied: {
    title: "Access denied",
    body: "You don't have permission to sign in. If you think this is a mistake, contact us.",
  },
  Verification: {
    title: "Link expired",
    body: "This sign-in link has already been used or it expired. Request a new one and try again.",
  },
  OAuthAccountNotLinked: {
    title: "Account not linked",
    body: "This email is already associated with a different sign-in method. Use the original method to sign in.",
  },
  Default: {
    title: "Sign-in failed",
    body: "Something went wrong while signing you in. Try again, or use a different method.",
  },
};

function resolve(code: string | undefined): { title: string; body: string } {
  if (code && code in MESSAGES) {
    return MESSAGES[code as AuthErrorCode];
  }
  return MESSAGES.Default;
}

export default async function AuthErrorPage({
  searchParams,
}: {
  searchParams: Promise<{ error?: string }>;
}) {
  const sp = await searchParams;
  const { title, body } = resolve(sp.error);

  return (
    <div className="proto-page-narrow">
      <h1>
        <span className="proto-inline-icon" aria-hidden>
          <AlertCircle size={20} />
        </span>{" "}
        {title}
      </h1>
      <p className="proto-dek">{body}</p>
      <p className="proto-empty proto-empty-spaced">
        <Link href="/login">Back to sign in</Link>
      </p>
    </div>
  );
}
