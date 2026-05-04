import Link from "next/link";

import { submitAndRedirect } from "@/lib/actions/submission";
import { auth } from "@/lib/auth";
import { getCurrentUser } from "@/lib/auth-shim";
import { MarkdownEditor } from "@/components/prototype/MarkdownEditor";

const SUBMISSION_TYPES = [
  ["news", "News"],
  ["tip", "Tip"],
  ["tutorial", "Tutorial"],
  ["course", "Course"],
  ["article", "Article"],
  ["podcast", "Podcast"],
  ["interview", "Interview"],
  ["tool", "Tool"],
  ["discussion", "Discussion (text)"],
] as const;

export default async function SubmitPage({
  searchParams,
}: {
  searchParams: Promise<{ mode?: string; error?: string; as?: string }>;
}) {
  const sp = await searchParams;
  const session = await auth();
  const devUser = getCurrentUser(sp);
  const isSignedIn = Boolean(session?.user) || Boolean(devUser);

  if (!isSignedIn) {
    return (
      <div className="proto-page-narrow">
        <h1>Submit</h1>
        <p className="proto-dek">
          <Link href="/login">Sign in</Link> to submit. Or append{" "}
          <code>?as=ada</code> to simulate a session in dev.
        </p>
      </div>
    );
  }

  const mode = sp.mode === "text" ? "text" : "link";

  return (
    <div className="proto-page-narrow">
      <h1>Submit</h1>
      <p className="proto-dek">
        Share something useful for builders working with AI tools. Tags are
        assigned automatically — you focus on the post. New users go through
        staff review until they have 50 karma or two approved submissions.
      </p>

      {sp.error && (
        <div className="proto-empty proto-empty-spaced" role="alert">
          {sp.error === "validation"
            ? "That input didn't pass validation — try again."
            : sp.error === "rate"
              ? "You're submitting too fast. Try again in a minute."
              : sp.error === "unauth"
                ? "You need to be signed in."
                : sp.error === "locked"
                  ? "This account is locked."
                  : "Something went wrong."}
        </div>
      )}

      <nav className="proto-tabs" aria-label="Submission mode">
        <Link
          href="/submit?mode=link"
          aria-current={mode === "link" ? "page" : undefined}
        >
          Link
        </Link>
        <Link
          href="/submit?mode=text"
          aria-current={mode === "text" ? "page" : undefined}
        >
          Text post
        </Link>
      </nav>

      <form className="proto-form" action={submitAndRedirect}>
        <label>
          Type
          <select name="type" defaultValue={mode === "text" ? "discussion" : "news"} required>
            {SUBMISSION_TYPES.map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </label>

        {mode === "link" && (
          <label>
            URL
            <input
              name="url"
              type="url"
              placeholder="https://"
              required
            />
            <span className="help">
              Pasting a previously-submitted URL (last 30 days) redirects you
              to the existing post.
            </span>
          </label>
        )}

        <label>
          Title
          <input
            name="title"
            type="text"
            placeholder="A clear description of the link"
            maxLength={120}
            required
          />
          <span className="help">Max 120 chars. No clickbait.</span>
        </label>

        {mode === "text" && (
          <label>
            Body
            <MarkdownEditor
              name="text"
              rows={6}
              maxLength={40000}
              placeholder="Markdown supported (subset)…"
              required
            />
            <span className="help">
              Allowed: paragraphs, italic, bold, links, code, lists, blockquote,
              strikethrough. No headings, images, or tables.
            </span>
          </label>
        )}

        <button type="submit" className="proto-btn-primary">
          Post
        </button>
      </form>
    </div>
  );
}
