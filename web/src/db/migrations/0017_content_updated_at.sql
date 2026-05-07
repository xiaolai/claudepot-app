-- 0017_content_updated_at — track post-window edits.
--
-- Background: editSubmission / editComment historically rewrote the
-- body silently within a 5-minute window — fine for human typo fixes
-- because no one had read the post yet. Slice-2's update API exposes
-- edits to PAT-authenticated bot clients (is_agent / role IN
-- ("system","staff")) without that window, which means out-of-window
-- edits CAN reach a reader who already saw the original. The badge
-- "edited HH:MM" is the trust signal; updated_at is its source.
--
-- Semantics:
--   - NULL          → never edited (or only edited within the human
--                     5-min window — those edits stay silent on
--                     purpose; same UX as today).
--   - timestamptz   → most recent edit that happened OUTSIDE the
--                     window. UI surfaces "edited <relative>".
--
-- Backfill: none. Existing rows are NULL → "never edited", which is
-- the truthful default — we cannot reconstruct historical edits.
--
-- Indexes: none. The column is read on the post-detail page along
-- with the row itself, never used as a filter or sort key.

ALTER TABLE "submissions"
  ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;
--> statement-breakpoint

ALTER TABLE "comments"
  ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;
