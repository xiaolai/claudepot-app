import Link from "next/link";
import { Mail } from "lucide-react";

export default function VerifyRequestPage() {
  return (
    <div className="proto-page-narrow">
      <h1>
        <span className="proto-inline-icon" aria-hidden>
          <Mail size={20} />
        </span>{" "}
        Check your email
      </h1>
      <p className="proto-dek">
        We sent a one-click sign-in link to your inbox. Open it on this
        device to finish signing in. The link expires in 24 hours and can
        only be used once.
      </p>
      <p className="proto-empty proto-empty-spaced">
        Nothing in your inbox? Check spam, or{" "}
        <Link href="/login">request another link</Link>.
      </p>
    </div>
  );
}
