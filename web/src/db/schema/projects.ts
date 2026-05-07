/**
 * Projects + their joins.
 *
 * Curated collections of submissions. Project ↔ tag join (migration
 * 0010) mirrors `submission_tags` so the related-feed query can
 * JOIN projects → project_tags → submission_tags → submissions
 * cleanly.
 */

import {
  index,
  integer,
  pgTable,
  primaryKey,
  text,
  timestamp,
  uuid,
} from "drizzle-orm/pg-core";

import { submissions, tags } from "./content";
import { users } from "./users";

export const projects = pgTable("projects", {
  id: uuid("id").primaryKey().defaultRandom(),
  slug: text("slug").notNull().unique(),
  name: text("name").notNull(),
  blurb: text("blurb"),
  ownerId: uuid("owner_id")
    .notNull()
    .references(() => users.id),
  // GitHub-sourced metadata (migration 0005). Refresh via
  // `pnpm tsx scripts/sync-projects.ts`.
  repoUrl: text("repo_url"),
  siteUrl: text("site_url"),
  primaryLanguage: text("primary_language"),
  stars: integer("stars").notNull().default(0),
  updatedAt: timestamp("updated_at", { withTimezone: true }),
  // README snapshot + editorial lede (migration 0010). README is
  // pulled from GitHub via the sync script; editorial is hand-authored.
  readmeMd: text("readme_md"),
  editorialMd: text("editorial_md"),
  createdAt: timestamp("created_at", { withTimezone: true }).notNull().defaultNow(),
});

export const projectSubmissions = pgTable(
  "project_submissions",
  {
    projectId: uuid("project_id")
      .notNull()
      .references(() => projects.id, { onDelete: "cascade" }),
    submissionId: uuid("submission_id")
      .notNull()
      .references(() => submissions.id, { onDelete: "cascade" }),
  },
  (t) => [primaryKey({ columns: [t.projectId, t.submissionId] })],
);

export const projectTags = pgTable(
  "project_tags",
  {
    projectId: uuid("project_id")
      .notNull()
      .references(() => projects.id, { onDelete: "cascade" }),
    tagSlug: text("tag_slug")
      .notNull()
      .references(() => tags.slug, { onDelete: "cascade" }),
  },
  (t) => [
    primaryKey({ columns: [t.projectId, t.tagSlug] }),
    index("idx_project_tags_tag").on(t.tagSlug, t.projectId),
  ],
);
