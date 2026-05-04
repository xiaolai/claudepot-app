/**
 * Insert one approved discussion submission whose body exercises
 * every renderMarkdown surface: headings, inline markup, lists,
 * tables (basic + aligned), images from the public web, multiple
 * blockquotes (so the hue rotator visibly rotates), GFM-style
 * alert callouts (NOTE / TIP / IMPORTANT / WARNING / CAUTION),
 * fenced code in several languages, a tall code block to show the
 * max-height scroll, and several mermaid diagram types (flowchart,
 * sequence, class, ER, state, gantt, pie, journey).
 *
 * Backdated 40 days so it doesn't appear on the first ~2 pages of
 * the home / new feeds.
 *
 *   pnpm tsx --env-file=.env.local scripts/seed-markdown-rich-post.ts
 *   pnpm tsx --env-file=.env.local scripts/seed-markdown-rich-post.ts <username>
 *
 * Default author is `ada` (system staff). Pass an alternate username
 * as argv[2] to attribute the post to someone else. Re-running mints
 * a new row.
 */

import { desc, eq } from "drizzle-orm";

import { db } from "@/db/client";
import { submissions, users } from "@/db/schema";

const BODY = `**The full markdown smoke test.** This post exercises every element \`renderMarkdown\` now supports — headings, inline markup, lists, tables, images, blockquotes (with hue rotation), GFM alert callouts, fenced code, and a stack of mermaid diagram types. If anything below renders incorrectly, the read path or the typography rules need attention.

# Heading 1 (user-content; demoted vs the page title)
## Heading 2
### Heading 3
#### Heading 4
##### Heading 5
###### Heading 6

A paragraph after the heading stack to verify the rhythm between heading and prose.

## Inline markup

Plain text. *Italic with em.* **Strong / bold.** ~~Strikethrough.~~ Inline \`code\` with backticks. A [hyperlink to /about](/about), a [hyperlink with title](/about "Hover for tooltip"), and an [external link](https://example.com).

Hard line break (two trailing spaces):
this should be on a new line within the same paragraph.

A soft paragraph break:

new paragraph entirely.

Combined run: ***bold italic***, **bold with \`inline code\` inside**, [a link with **bold** inside](/), and ~~strikethrough containing *italic*~~.

---

## Lists — unordered

- First top-level item.
- Second top-level item with a longer body so we can see the gutter alignment when text wraps onto a second line. Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt.
- Item with **bold** and \`inline code\`.
- Nested:
  - second level
  - second level with even more text to force wrap, lorem ipsum dolor sit amet
    - third level (deepest the renderer should reasonably handle)
    - another third-level
  - back to second
- Back to top.

## Lists — ordered (with double-digit alignment)

1. First ordered item.
2. Second item, longer body, so the wrap alignment is visible against the gutter — *italic for emphasis* — and a [link](/) tucked in the middle.
3. Third item with nested content:
   1. inner one
   2. inner two
   3. inner three
4. Items 4–10 fill in to demonstrate the gutter handles double digits:
5. fifth
6. sixth
7. seventh
8. eighth
9. ninth
10. tenth — note the \`10.\` sits in the same gutter column as \`9.\` because numbers are right-aligned.

## Lists — mixed

1. Ordered top-level with an unordered child:
   - bullet child A
   - bullet child B
2. Ordered with an ordered child:
   1. inner A
   2. inner B
3. Bullets nested inside an ordered list inside a bullet:
   - bullet
     1. ordered
        - bullet again
        - and one more

---

## Tables

Basic:

| Column A | Column B | Column C |
|----------|----------|----------|
| one      | two      | three    |
| four     | five     | six      |
| seven    | eight    | nine     |

With alignment (left / center / right):

| Left aligned | Center aligned | Right aligned |
|:-------------|:--------------:|--------------:|
| left         | center         | right         |
| short        | a longer cell  | 1234          |
| —            | —              | 999           |

Wide table to exercise horizontal scroll inside the column:

| ID | Title                              | Type        | Tags                              | Author    | Score | Status   |
|----|------------------------------------|-------------|-----------------------------------|-----------|-------|----------|
| 1  | Long submission title goes here    | tutorial    | claude, agent, mcp, prompts       | @ada      | 42    | approved |
| 2  | Another long row to force overflow | tool        | cli, eval, openrouter, typescript | @kai      | 17    | approved |
| 3  | Yet another row                    | release     | gpt, anthropic, openai            | @miro     | 9     | pending  |

---

## Blockquotes — hue rotation

Each blockquote below hashes its content into one of six accent-related OKLCH hues. Successive quotes rotate visibly:

> First: a single-line quote. Short and sharp.

> Second: three lines, but only one paragraph. The renderer treats consecutive lines inside a single \`>\` block as one paragraph unless a blank \`>\` line separates them.

> Third: multi-paragraph quote.
>
> The blank \`>\` line above starts a new paragraph inside the same blockquote. Block rhythm should give a \`--sp-16\` gap.
>
> A third paragraph for good measure, with a [link inside the quote](/) and **bold inside the quote**.

> Fourth: contains a \`code span\`, *italics*, and ~~struck-through text~~ — all of which should respect the muted color of the quote.

> Fifth: nested quote test.
>
> > Inner quote (limitation: the regex matcher only colors the outermost; the inner stays at the default accent hue).
>
> Back to outer quote.

> Sixth: another sibling, to land on yet a different hue bucket.

## Blockquotes — GFM alert callouts

Below: each of the five GitHub Flavored Markdown alert types. The \`[!TYPE]\` marker on the first line is stripped server-side; the type drives the left rule color, the tinted background, and the uppercase label.

> [!NOTE]
> Highlights information that users should take into account, even when skimming. Renders with the blue accent.

> [!TIP]
> Optional information to help a user be more successful. Renders with the green accent.

> [!IMPORTANT]
> Crucial information necessary for users to succeed. Renders with the purple accent.

> [!WARNING]
> Critical content demanding immediate user attention due to potential risks. Renders with the amber accent.

> [!CAUTION]
> Negative potential consequences of an action. Renders with the red-orange accent.

A multi-paragraph callout:

> [!NOTE]
> First paragraph of the callout.
>
> Second paragraph of the same callout. The label only appears once at the top.
>
> Third paragraph, with a \`code span\` and a [link](/).

---

## Images (from the public web)

Inline image (small, sits in a paragraph): ![tiny seed](https://picsum.photos/seed/claudepot-inline/40/40) at 40×40.

Standalone image (block-level, sits alone in its paragraph):

![Lorem Picsum 600×300](https://picsum.photos/seed/claudepot-rich-post/600/300)

A second standalone image:

![Lorem Picsum 800×450](https://picsum.photos/seed/claudepot-second/800/450 "Hover title set on the image")

---

## Fenced code blocks

TypeScript:

\`\`\`ts
type Submission = {
  id: string;
  authorId: string;
  title: string;
  text: string | null;
  createdAt: Date;
};

function rank(s: Submission): number {
  return s.title.length;
}
\`\`\`

Bash:

\`\`\`bash
#!/usr/bin/env bash
set -euo pipefail
for f in "$@"; do
  if [[ -f "$f" ]]; then
    wc -l "$f"
  fi
done
\`\`\`

JSON:

\`\`\`json
{
  "name": "claudepot",
  "tags": ["ai", "claude", "tools"],
  "metrics": { "submissions": 42, "score": 1337 }
}
\`\`\`

Plain (no language tag):

\`\`\`
just text — no syntax tag, no syntax highlighting,
the gutter still numbers each line,
and the copy button still works.
\`\`\`

## Tall code block (scroll test)

\`\`\`ts
type Tag =
  | "language-model"
  | "agent-framework"
  | "ide-integration"
  | "code-search"
  | "embedding"
  | "fine-tuning"
  | "evaluation"
  | "guardrails"
  | "router"
  | "telemetry"
  | "deployment"
  | "ux-pattern"
  | "prompt-engineering"
  | "synthetic-data"
  | "labeling"
  | "human-loop";

interface Submission {
  id: string;
  title: string;
  url: string | null;
  text: string | null;
  type:
    | "news"
    | "tutorial"
    | "podcast"
    | "tool"
    | "discussion";
  tags: readonly Tag[];
  authorId: string;
  score: number;
  publishedAt: Date | null;
  createdAt: Date;
}

function classify(s: Submission): "feed" | "firehose" | "queue" {
  const ageDays = (Date.now() - s.createdAt.getTime()) / 86_400_000;
  if (s.score >= 50 && ageDays < 30) return "feed";
  if (s.score < 5 && ageDays > 7) return "firehose";
  return "queue";
}
\`\`\`

---

## Mermaid diagrams

Flowchart:

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

Sequence:

\`\`\`mermaid
sequenceDiagram
  participant U as User
  participant W as Web app
  participant DB as Postgres
  U->>W: POST /api/v1/submissions
  W->>W: authenticate token
  W->>DB: insert submissions row
  DB-->>W: row id
  W-->>U: 201 Created (id, url)
\`\`\`

Class:

\`\`\`mermaid
classDiagram
  class Submission {
    +UUID id
    +String title
    +String url
    +String text
    +Date createdAt
    +rank() Number
  }
  class Comment {
    +UUID id
    +UUID submissionId
    +String body
    +Date createdAt
  }
  Submission "1" --> "0..*" Comment : has
\`\`\`

Entity-relationship:

\`\`\`mermaid
erDiagram
  USER ||--o{ SUBMISSION : authors
  USER ||--o{ COMMENT : writes
  SUBMISSION ||--o{ COMMENT : receives
  SUBMISSION ||--o{ VOTE : accumulates
  USER ||--o{ VOTE : casts
  USER {
    uuid id PK
    string username
    int karma
    timestamptz createdAt
  }
  SUBMISSION {
    uuid id PK
    uuid authorId FK
    string title
    string url
    text body
    int score
  }
\`\`\`

State diagram (submission lifecycle):

\`\`\`mermaid
stateDiagram-v2
  [*] --> Pending : submitted
  Pending --> Approved : AI auto-approve
  Pending --> Rejected : AI rejects
  Pending --> Queue : low confidence
  Queue --> Approved : staff approves
  Queue --> Rejected : staff rejects
  Approved --> Deleted : author deletes
  Rejected --> [*]
  Deleted --> [*]
\`\`\`

Gantt (release timeline):

\`\`\`mermaid
gantt
  title Read-path hardening sprint
  dateFormat  YYYY-MM-DD
  section Markdown
  Render bodies + comments     :done,    a1, 2026-04-29, 1d
  Code-block decoration         :done,    a2, after a1, 1d
  Mermaid hydrator              :done,    a3, after a2, 1d
  GFM alerts + tables + images  :active,  a4, 2026-05-03, 2d
  section Quality
  Audit + verify loop           :crit,    b1, 2026-05-02, 1d
\`\`\`

Pie:

\`\`\`mermaid
pie title Submissions by routing destination
  "Feed"        : 42
  "Firehose"    : 31
  "Human queue" : 7
\`\`\`

User journey:

\`\`\`mermaid
journey
  title Reader journey through a discussion post
  section Arrive
    Land on /post/[id]: 5: Reader
    See title + meta: 5: Reader
  section Read body
    Scan inline markup: 4: Reader
    Read code block: 5: Reader
    Hover copy button: 5: Reader
  section Engage
    Upvote: 4: Reader
    Save: 4: Reader
    Comment: 3: Reader
\`\`\`

---

## What the sanitizer still strips

The renderer's allowlist now includes headings, tables, images, and hr — but raw HTML, scripts, and inline style attributes are still off-limits. The lines below SHOULD NOT render visually:

<div style="background: red; padding: 1rem;">if you see a red box, the sanitizer failed</div>

<script>alert("if you see this, the sanitizer FAILED catastrophically")</script>

End of test post. If everything above renders correctly, the markdown read path is solid.
`;

async function main() {
  const username = process.argv[2] ?? "ada";

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
    process.exit(1);
  }

  const old = new Date(Date.now() - 40 * 86_400_000);
  const [row] = await db
    .insert(submissions)
    .values({
      authorId: author.id,
      type: "discussion",
      title:
        "(test) full markdown rendering — headings, tables, images, callouts, mermaid",
      url: null,
      text: BODY,
      state: "approved",
      publishedAt: old,
      createdAt: old,
    })
    .returning({ id: submissions.id });

  const siteUrl =
    process.env.NEXT_PUBLIC_SITE_URL ?? "https://claudepot.com";
  console.log(
    `Inserted submission ${row.id} attributed to @${author.username}.`,
  );
  console.log(`Local: http://localhost:3000/post/${row.id}`);
  console.log(`Prod:  ${siteUrl}/post/${row.id}`);
  process.exit(0);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
