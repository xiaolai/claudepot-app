-- Enable Postgres extensions used by the schema.
-- citext powers case-insensitive uniqueness on users.username and users.email.
CREATE EXTENSION IF NOT EXISTS "citext";--> statement-breakpoint
CREATE TYPE "public"."ai_decision" AS ENUM('approve', 'reject', 'escalate');--> statement-breakpoint
CREATE TYPE "public"."content_state" AS ENUM('pending', 'approved', 'rejected');--> statement-breakpoint
CREATE TYPE "public"."flag_status" AS ENUM('open', 'resolved');--> statement-breakpoint
CREATE TYPE "public"."moderation_action" AS ENUM('lock', 'unlist', 'delete', 'restore', 'dismiss_flag', 'lock_user', 'approve', 'reject', 'delete_hard');--> statement-breakpoint
CREATE TYPE "public"."notification_kind" AS ENUM('comment_reply', 'submission_reply', 'moderation', 'mention');--> statement-breakpoint
CREATE TYPE "public"."submission_type" AS ENUM('news', 'tip', 'tutorial', 'course', 'article', 'podcast', 'interview', 'tool', 'discussion');--> statement-breakpoint
CREATE TYPE "public"."target_type" AS ENUM('submission', 'comment');--> statement-breakpoint
CREATE TYPE "public"."user_role" AS ENUM('user', 'staff', 'locked', 'system');--> statement-breakpoint
CREATE TABLE "accounts" (
	"user_id" uuid NOT NULL,
	"type" text NOT NULL,
	"provider" text NOT NULL,
	"provider_account_id" text NOT NULL,
	"refresh_token" text,
	"access_token" text,
	"expires_at" integer,
	"token_type" text,
	"scope" text,
	"id_token" text,
	"session_state" text,
	CONSTRAINT "accounts_provider_provider_account_id_pk" PRIMARY KEY("provider","provider_account_id")
);
--> statement-breakpoint
CREATE TABLE "ai_decisions" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"target_type" "target_type" NOT NULL,
	"target_id" uuid NOT NULL,
	"decision" "ai_decision" NOT NULL,
	"confidence" numeric(3, 2) NOT NULL,
	"model_id" text NOT NULL,
	"reasoning" text NOT NULL,
	"prompt_hash" text NOT NULL,
	"cost_usd" numeric(10, 6) NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "comments" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"author_id" uuid NOT NULL,
	"submission_id" uuid NOT NULL,
	"parent_id" uuid,
	"body" text NOT NULL,
	"state" "content_state" DEFAULT 'approved' NOT NULL,
	"score" integer DEFAULT 0 NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"deleted_at" timestamp with time zone
);
--> statement-breakpoint
CREATE TABLE "flags" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"reporter_id" uuid NOT NULL,
	"target_type" "target_type" NOT NULL,
	"target_id" uuid NOT NULL,
	"reason" text NOT NULL,
	"status" "flag_status" DEFAULT 'open' NOT NULL,
	"resolved_by" uuid,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"resolved_at" timestamp with time zone
);
--> statement-breakpoint
CREATE TABLE "metrics_daily" (
	"day" date PRIMARY KEY NOT NULL,
	"submissions_total" integer DEFAULT 0 NOT NULL,
	"comments_total" integer DEFAULT 0 NOT NULL,
	"votes_total" integer DEFAULT 0 NOT NULL,
	"signups_total" integer DEFAULT 0 NOT NULL,
	"active_users_24h" integer DEFAULT 0 NOT NULL
);
--> statement-breakpoint
CREATE TABLE "moderation_log" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"staff_id" uuid NOT NULL,
	"action" "moderation_action" NOT NULL,
	"target_type" "target_type",
	"target_id" uuid,
	"note" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "moderation_overrides" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"target_type" "target_type" NOT NULL,
	"target_id" uuid NOT NULL,
	"ai_decision_id" uuid,
	"staff_id" uuid NOT NULL,
	"override" "ai_decision" NOT NULL,
	"note" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "notifications" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"user_id" uuid NOT NULL,
	"kind" "notification_kind" NOT NULL,
	"payload" jsonb NOT NULL,
	"read_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "project_submissions" (
	"project_id" uuid NOT NULL,
	"submission_id" uuid NOT NULL,
	CONSTRAINT "project_submissions_project_id_submission_id_pk" PRIMARY KEY("project_id","submission_id")
);
--> statement-breakpoint
CREATE TABLE "projects" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"slug" text NOT NULL,
	"name" text NOT NULL,
	"blurb" text,
	"owner_id" uuid NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "projects_slug_unique" UNIQUE("slug")
);
--> statement-breakpoint
CREATE TABLE "saves" (
	"user_id" uuid NOT NULL,
	"submission_id" uuid NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "saves_user_id_submission_id_pk" PRIMARY KEY("user_id","submission_id")
);
--> statement-breakpoint
CREATE TABLE "sessions" (
	"session_token" text PRIMARY KEY NOT NULL,
	"user_id" uuid NOT NULL,
	"expires" timestamp with time zone NOT NULL
);
--> statement-breakpoint
CREATE TABLE "submission_tags" (
	"submission_id" uuid NOT NULL,
	"tag_slug" text NOT NULL,
	CONSTRAINT "submission_tags_submission_id_tag_slug_pk" PRIMARY KEY("submission_id","tag_slug")
);
--> statement-breakpoint
CREATE TABLE "submissions" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"author_id" uuid NOT NULL,
	"type" "submission_type" NOT NULL,
	"title" text NOT NULL,
	"url" text,
	"text" text,
	"state" "content_state" DEFAULT 'pending' NOT NULL,
	"score" integer DEFAULT 0 NOT NULL,
	"reading_time_min" integer,
	"podcast_meta" jsonb,
	"tool_meta" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"published_at" timestamp with time zone,
	"deleted_at" timestamp with time zone
);
--> statement-breakpoint
CREATE TABLE "tags" (
	"slug" text PRIMARY KEY NOT NULL,
	"name" text NOT NULL,
	"tagline" text,
	"sort_order" integer DEFAULT 0 NOT NULL
);
--> statement-breakpoint
CREATE TABLE "user_email_prefs" (
	"user_id" uuid PRIMARY KEY NOT NULL,
	"digest_weekly" boolean DEFAULT true NOT NULL,
	"notify_replies" boolean DEFAULT true NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "user_hidden_submissions" (
	"user_id" uuid NOT NULL,
	"submission_id" uuid NOT NULL,
	"hidden_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "user_hidden_submissions_user_id_submission_id_pk" PRIMARY KEY("user_id","submission_id")
);
--> statement-breakpoint
CREATE TABLE "user_tag_mutes" (
	"user_id" uuid NOT NULL,
	"tag_slug" text NOT NULL,
	"muted_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "user_tag_mutes_user_id_tag_slug_pk" PRIMARY KEY("user_id","tag_slug")
);
--> statement-breakpoint
CREATE TABLE "users" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"username" "citext" NOT NULL,
	"email" "citext" NOT NULL,
	"email_verified" timestamp with time zone,
	"avatar_url" text,
	"bio" text,
	"role" "user_role" DEFAULT 'user' NOT NULL,
	"is_agent" boolean DEFAULT false NOT NULL,
	"karma" integer DEFAULT 0 NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "verification_tokens" (
	"identifier" text NOT NULL,
	"token" text NOT NULL,
	"expires" timestamp with time zone NOT NULL,
	CONSTRAINT "verification_tokens_identifier_token_pk" PRIMARY KEY("identifier","token")
);
--> statement-breakpoint
CREATE TABLE "votes" (
	"user_id" uuid NOT NULL,
	"submission_id" uuid NOT NULL,
	"value" integer NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "votes_user_id_submission_id_pk" PRIMARY KEY("user_id","submission_id")
);
--> statement-breakpoint
ALTER TABLE "accounts" ADD CONSTRAINT "accounts_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "comments" ADD CONSTRAINT "comments_author_id_users_id_fk" FOREIGN KEY ("author_id") REFERENCES "public"."users"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "comments" ADD CONSTRAINT "comments_submission_id_submissions_id_fk" FOREIGN KEY ("submission_id") REFERENCES "public"."submissions"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "flags" ADD CONSTRAINT "flags_reporter_id_users_id_fk" FOREIGN KEY ("reporter_id") REFERENCES "public"."users"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "flags" ADD CONSTRAINT "flags_resolved_by_users_id_fk" FOREIGN KEY ("resolved_by") REFERENCES "public"."users"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "moderation_log" ADD CONSTRAINT "moderation_log_staff_id_users_id_fk" FOREIGN KEY ("staff_id") REFERENCES "public"."users"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "moderation_overrides" ADD CONSTRAINT "moderation_overrides_ai_decision_id_ai_decisions_id_fk" FOREIGN KEY ("ai_decision_id") REFERENCES "public"."ai_decisions"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "moderation_overrides" ADD CONSTRAINT "moderation_overrides_staff_id_users_id_fk" FOREIGN KEY ("staff_id") REFERENCES "public"."users"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "notifications" ADD CONSTRAINT "notifications_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "project_submissions" ADD CONSTRAINT "project_submissions_project_id_projects_id_fk" FOREIGN KEY ("project_id") REFERENCES "public"."projects"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "project_submissions" ADD CONSTRAINT "project_submissions_submission_id_submissions_id_fk" FOREIGN KEY ("submission_id") REFERENCES "public"."submissions"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "projects" ADD CONSTRAINT "projects_owner_id_users_id_fk" FOREIGN KEY ("owner_id") REFERENCES "public"."users"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "saves" ADD CONSTRAINT "saves_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "saves" ADD CONSTRAINT "saves_submission_id_submissions_id_fk" FOREIGN KEY ("submission_id") REFERENCES "public"."submissions"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "sessions" ADD CONSTRAINT "sessions_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "submission_tags" ADD CONSTRAINT "submission_tags_submission_id_submissions_id_fk" FOREIGN KEY ("submission_id") REFERENCES "public"."submissions"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "submission_tags" ADD CONSTRAINT "submission_tags_tag_slug_tags_slug_fk" FOREIGN KEY ("tag_slug") REFERENCES "public"."tags"("slug") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "submissions" ADD CONSTRAINT "submissions_author_id_users_id_fk" FOREIGN KEY ("author_id") REFERENCES "public"."users"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "user_email_prefs" ADD CONSTRAINT "user_email_prefs_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "user_hidden_submissions" ADD CONSTRAINT "user_hidden_submissions_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "user_hidden_submissions" ADD CONSTRAINT "user_hidden_submissions_submission_id_submissions_id_fk" FOREIGN KEY ("submission_id") REFERENCES "public"."submissions"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "user_tag_mutes" ADD CONSTRAINT "user_tag_mutes_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "user_tag_mutes" ADD CONSTRAINT "user_tag_mutes_tag_slug_tags_slug_fk" FOREIGN KEY ("tag_slug") REFERENCES "public"."tags"("slug") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "votes" ADD CONSTRAINT "votes_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "votes" ADD CONSTRAINT "votes_submission_id_submissions_id_fk" FOREIGN KEY ("submission_id") REFERENCES "public"."submissions"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "idx_ai_decisions_target" ON "ai_decisions" USING btree ("target_type","target_id","created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "idx_comments_submission_created" ON "comments" USING btree ("submission_id","created_at");--> statement-breakpoint
CREATE INDEX "idx_comments_parent" ON "comments" USING btree ("parent_id");--> statement-breakpoint
CREATE INDEX "idx_comments_author" ON "comments" USING btree ("author_id");--> statement-breakpoint
CREATE INDEX "idx_flags_open" ON "flags" USING btree ("target_type","target_id","status");--> statement-breakpoint
CREATE INDEX "idx_flags_reporter" ON "flags" USING btree ("reporter_id");--> statement-breakpoint
CREATE INDEX "idx_moderation_log_created" ON "moderation_log" USING btree ("created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "idx_moderation_log_staff" ON "moderation_log" USING btree ("staff_id");--> statement-breakpoint
CREATE INDEX "idx_notifications_user_unread" ON "notifications" USING btree ("user_id","created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "idx_submission_tags_tag" ON "submission_tags" USING btree ("tag_slug","submission_id");--> statement-breakpoint
CREATE INDEX "idx_submissions_state_created" ON "submissions" USING btree ("state","created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "idx_submissions_state_score" ON "submissions" USING btree ("state","score" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "idx_submissions_author" ON "submissions" USING btree ("author_id");--> statement-breakpoint
CREATE UNIQUE INDEX "idx_users_username" ON "users" USING btree ("username");--> statement-breakpoint
CREATE UNIQUE INDEX "idx_users_email" ON "users" USING btree ("email");