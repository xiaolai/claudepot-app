-- 0010_project_readme_tags — give projects a real long-form body and a
-- proper tag-bound related feed.
--
-- Background: pre-this-migration, /projects/[slug] showed only the
-- name/tagline/stat strip from 0005, plus a "Related on shannon"
-- section that ILIKE-matched the slug or name against submission
-- titles/urls (src/db/queries.ts:556). That's strictly less than what
-- GitHub already shows, and the related-feed match is unreliable
-- (e.g., "claudepot-app" hits almost nothing while a `claudepot` tag
-- would match cleanly).
--
-- This migration adds:
--   * projects.readme_md     — README snapshot, populated by the
--                              GitHub sync script (sync-projects.ts).
--   * projects.editorial_md  — hand-authored editorial blurb,
--                              displayed as the lede above the README.
--   * project_tags           — many-to-many between projects and tags,
--                              mirroring the submission_tags shape.
--                              Drives the "Related on shannon" feed:
--                              project.tags ∩ submission.tags.
--
-- No data backfill: readme_md / editorial_md start NULL; project_tags
-- starts empty. The detail page renders cleanly in either case.

ALTER TABLE "projects" ADD COLUMN IF NOT EXISTS "readme_md"    text;--> statement-breakpoint
ALTER TABLE "projects" ADD COLUMN IF NOT EXISTS "editorial_md" text;--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "project_tags" (
  "project_id" uuid NOT NULL REFERENCES "projects"("id") ON DELETE CASCADE,
  "tag_slug"   text NOT NULL REFERENCES "tags"("slug")    ON DELETE CASCADE,
  PRIMARY KEY ("project_id", "tag_slug")
);--> statement-breakpoint

-- Reverse-direction lookup: "all projects with this tag".
CREATE INDEX IF NOT EXISTS "idx_project_tags_tag"
  ON "project_tags" ("tag_slug", "project_id");
