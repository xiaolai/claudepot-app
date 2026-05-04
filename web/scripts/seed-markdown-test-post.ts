/**
 * Insert one approved discussion submission whose body exercises the
 * markdown read path: bold/italic/code, fenced TS code, fenced mermaid,
 * lists, blockquote, link, strikethrough.
 *
 * Backdated 35 days so it doesn't surface in the first ~2 pages of the
 * hot or new feeds.
 *
 *   pnpm tsx --env-file=.env.local scripts/seed-markdown-test-post.ts
 *   pnpm tsx --env-file=.env.local scripts/seed-markdown-test-post.ts <username>
 *
 * Default author is `lixiaolai` (staff fixture user). Pass an alternate
 * username as argv[2] to attribute the post to someone else.
 *
 * Prints the resulting `/post/<id>` URL. Re-running mints a new row.
 */

import { desc, eq } from "drizzle-orm";

import { db } from "@/db/client";
import { submissions, users } from "@/db/schema";

const BODY = `**Markdown rendering smoke test.**

Inline markup: *italic*, **bold**, ~~strikethrough~~, \`inline code\`, and a [link to the homepage](/).

A short list of what should render:

- paragraphs with line breaks
- ordered and unordered lists
- inline code and fenced code blocks
- block quotes
- links with rel/target hardened by the sanitizer

Ordered:

1. Authors compose markdown via the editor.
2. The same renderer runs server-side at submit time and read time.
3. Sanitize-html strips anything outside the allowlist before HTML reaches the browser.

> Quotes too — a single block quote with one line of text.

> Second blockquote, different content. Each one hashes its body
> into one of six accent-derived OKLCH hues, so successive quotes
> rotate through the palette without any explicit position-based
> rule.

> A third, shorter quote.

> "Premature optimization is the root of all evil." — Donald Knuth, paraphrased.

Fenced TypeScript:

\`\`\`ts
type Submission = {
  id: string;
  title: string;
  text: string | null;
};

function rank(s: Submission): number {
  return s.title.length;
}
\`\`\`

Mermaid (NB: lives as a fenced code block in the source; whether it renders as a diagram depends on whether a mermaid client is mounted):

\`\`\`mermaid
flowchart LR
  A[Author] --> B[MarkdownEditor]
  B --> C[server action]
  C --> D[(Postgres)]
  D --> E[post page]
  E --> F[renderMarkdown]
  F --> G[sanitize-html]
  G --> H[reader]
\`\`\`

Final paragraph: if you're reading this and the bullets, code blocks, and links all render with formatting, the read-path fix is working.
`;

async function main() {
  const username = process.argv[2] ?? "lixiaolai";

  const [author] = await db
    .select({ id: users.id, username: users.username })
    .from(users)
    .where(eq(users.username, username))
    .limit(1);
  if (!author) {
    console.error(`No user found with username @${username}.`);
    const top = await db
      .select({ username: users.username, role: users.role })
      .from(users)
      .orderBy(desc(users.karma))
      .limit(10);
    if (top.length > 0) {
      console.error(`Available users (top 10 by karma):`);
      for (const r of top) console.error(`  - @${r.username}  [${r.role}]`);
    }
    console.error(`Re-run with one of those usernames as argv[2].`);
    process.exit(1);
  }

  const old = new Date(Date.now() - 35 * 86_400_000);
  const [row] = await db
    .insert(submissions)
    .values({
      authorId: author.id,
      type: "discussion",
      title: "(test) markdown rendering — code, mermaid, lists, links",
      url: null,
      text: BODY,
      state: "approved",
      publishedAt: old,
      createdAt: old,
    })
    .returning({ id: submissions.id });

  const siteUrl =
    process.env.NEXT_PUBLIC_SITE_URL ?? "https://claudepot.com";
  console.log(`Inserted submission ${row.id} attributed to @${author.username}.`);
  console.log(`Local: http://localhost:3000/post/${row.id}`);
  console.log(`Prod:  ${siteUrl}/post/${row.id}`);
  process.exit(0);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
