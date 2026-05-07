// Mirrors src-tauri/src/dto_cc_tips.rs.
//
// CC tips ledger — extracted from the user's CC binary, joined with
// `~/.claude.json::tipsHistory`, time-resolved via Claudepot
// snapshot diffs.

export type TipCategory =
  | "onboarding"
  | "workflow"
  | "shortcut"
  | "setup"
  | "memory-config"
  | "multi-session"
  | "ide"
  | "apps-extensions"
  | "plugins"
  | "experiments"
  | "billing"
  | "misc"
  | "internal";

export type TipSeenStatus = "seen" | "never-seen";

export interface TipLastSeen {
  relative: string;
  startup_count_when_seen: number;
  exact_unknown: boolean;
}

export interface RenderedTip {
  id: string;
  category: TipCategory;
  category_label: string;
  prose: string;
  prose_b: string | null;
  experiment_flag: string | null;
  condition_label: string | null;
  condition_label_b: string | null;
  cooldown_sessions: number | null;
  last_seen: TipLastSeen | null;
  trigger_summary: string;
  relevance_source: string | null;
  provider_agnostic: boolean;
  seen_status: TipSeenStatus;
}

export interface TipsCounts {
  all: number;
  seen: number;
  never_seen: number;
  active_experiments: number;
}

export interface TipsRender {
  catalog_version: string;
  extracted_at: number;
  partial: boolean;
  extracted_count: number;
  known_count: number;
  current_num_startups: number;
  tips: RenderedTip[];
  counts: TipsCounts;
}

export interface TipsRefreshResult {
  extracted_count: number;
  known_count: number;
  partial: boolean;
  catalog_version: string;
}
