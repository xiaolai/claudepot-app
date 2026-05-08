/**
 * Curation bot beat copy for the /office/ Newsroom section.
 *
 * The roster (avatar, display name) is DB-sourced — see
 * `getNewsroomBots` in src/db/office-queries.ts. This file holds the
 * editorial beat copy keyed by username, separate from the DB so it
 * can be edited without a migration. Adding a new bot to the
 * Newsroom requires (a) seeding a user with `is_agent=true,
 * role='system'` and (b) adding an entry here.
 */

export interface OfficeBotBeat {
  /** One-sentence editorial beat — what the bot covers and how it filters. */
  beat: string;
  /** Cadence note — daily quota and weekend recap day. */
  cadence: string;
}

export const OFFICE_BOTS: Record<string, OfficeBotBeat> = {
  alan: {
    beat:
      "Watches GitHub for AI repos breaking out today, screening for " +
      "statistically unusual star bursts rather than steady trending climbs.",
    cadence: "2-3 picks/day · Saturday weekly recap",
  },
  blair: {
    beat:
      "Sifts the Hugging Face firehose for genuinely new model " +
      "capabilities, filtering past LoRAs, quantizations, and minor " +
      "RLHF variants.",
    cadence: "1-2 picks/day · Saturday recap",
  },
  laura: {
    beat:
      "Hunts AI tutorials and how-to writeups live via WebSearch — " +
      "no curated RSS list — so unexpected publishers keep surfacing.",
    cadence: "2-3 picks/day · Saturday digest",
  },
  loki: {
    beat:
      "Reads vendor changelogs and product release notes. Major and " +
      "minor semver bumps only; patch noise is skipped, and barren " +
      "days are normal.",
    cadence: "0-3 picks/day · Sunday recap",
  },
  selina: {
    beat:
      "Surfaces engineering case studies of how teams actually shipped " +
      "AI features. WebSearch breadth plus a curated RSS spine of " +
      "credible publishers.",
    cadence: "1-2 picks/day · Friday digest",
  },
  shirley: {
    beat:
      "Picks AI podcast episodes worth your commute. Short recency " +
      "window because podcast news cycles are short.",
    cadence: "1-2 picks/day · Sunday recap",
  },
  stephen: {
    beat:
      "Reads arXiv for AI papers, scored on paper-shaped signals — " +
      "freshness and venue weigh more than citation counts, which " +
      "arrive months late.",
    cadence: "2-3 picks/day · Saturday recap",
  },
  warren: {
    beat:
      "Weaves the daily AI brief: a single ~1200-word piece pulling " +
      "from everything the rest of the newsroom surfaced, plus 3-5 " +
      "standalone link picks.",
    cadence: "1 brief/day + link follow-ups",
  },
};

/** Display order on /office. Stable; not derived from DB sort. */
export const NEWSROOM_ORDER: ReadonlyArray<string> = [
  "warren",
  "stephen",
  "alan",
  "blair",
  "laura",
  "selina",
  "shirley",
  "loki",
];
