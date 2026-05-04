-- Extends the projects table with GitHub-derived fields needed for
-- the /projects page rendering. Audit finding 5.1: the previous
-- mapProject() hardcoded empty values for repo_url, stars, etc.;
-- now they're real columns populated from `gh repo list xiaolai`.

ALTER TABLE "projects" ADD COLUMN IF NOT EXISTS "repo_url" text;--> statement-breakpoint
ALTER TABLE "projects" ADD COLUMN IF NOT EXISTS "site_url" text;--> statement-breakpoint
ALTER TABLE "projects" ADD COLUMN IF NOT EXISTS "primary_language" text;--> statement-breakpoint
ALTER TABLE "projects" ADD COLUMN IF NOT EXISTS "stars" integer NOT NULL DEFAULT 0;--> statement-breakpoint
ALTER TABLE "projects" ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;
