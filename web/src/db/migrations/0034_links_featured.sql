-- 0034_links_featured — set the editor's-picks strip on /links/.
--
-- Eight canonical entries spanning Anthropic, claude.ai, Claude Code,
-- MCP, Skills, Hugging Face, LMArena, OpenRouter — a "what is this?"
-- orientation row for first-time visitors before the link wall.
--
-- Idempotent: matches by URL, so re-running after a seed re-pass
-- restores the assignments. Run after 0033_links.sql + seed-links.ts.

UPDATE "links" SET "featured_rank" = 1,
  "featured_blurb" = 'Anthropic''s API documentation — the source of truth for everything Claude.'
  WHERE "url" = 'https://docs.anthropic.com';
--> statement-breakpoint

UPDATE "links" SET "featured_rank" = 2,
  "featured_blurb" = 'The Claude consumer chat — try the latest model in the browser.'
  WHERE "url" = 'https://claude.ai';
--> statement-breakpoint

UPDATE "links" SET "featured_rank" = 3,
  "featured_blurb" = 'Claude Code — Anthropic''s terminal coding agent. Hooks, plugins, skills, MCP.'
  WHERE "url" = 'https://code.claude.com/docs/en/overview';
--> statement-breakpoint

UPDATE "links" SET "featured_rank" = 4,
  "featured_blurb" = 'Model Context Protocol — the open standard for connecting AI agents to tools and data.'
  WHERE "url" = 'https://modelcontextprotocol.io';
--> statement-breakpoint

UPDATE "links" SET "featured_rank" = 5,
  "featured_blurb" = 'Anthropic''s Skills — capability bundles in markdown that ship with the Agent SDK.'
  WHERE "url" = 'https://github.com/anthropics/skills';
--> statement-breakpoint

UPDATE "links" SET "featured_rank" = 6,
  "featured_blurb" = 'Hugging Face — the model, dataset, and Space hub for the open-source ML world.'
  WHERE "url" = 'https://huggingface.co';
--> statement-breakpoint

UPDATE "links" SET "featured_rank" = 7,
  "featured_blurb" = 'LMArena — crowdsourced Elo ranking from blind pairwise human votes.'
  WHERE "url" = 'https://lmarena.ai';
--> statement-breakpoint

UPDATE "links" SET "featured_rank" = 8,
  "featured_blurb" = 'OpenRouter — one API across 300+ frontier and open models.'
  WHERE "url" = 'https://openrouter.ai';
