//! DTOs for the Global → Tips ledger. Pass-through shapes that
//! mirror `claudepot_core::cc_tips::catalog::TipsRender` minus a
//! few fields the GUI doesn't need.

use claudepot_core::cc_tips::catalog::{
    RenderedTip as CoreRenderedTip, SeenStatus as CoreSeenStatus, TipsCounts as CoreTipsCounts,
    TipsRender as CoreTipsRender,
};
use claudepot_core::cc_tips::categories::Category as CoreCategory;
use claudepot_core::cc_tips::history::LastSeen as CoreLastSeen;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct TipsRenderDto {
    pub catalog_version: String,
    pub extracted_at: i64,
    pub partial: bool,
    pub extracted_count: usize,
    pub known_count: usize,
    pub current_num_startups: u32,
    pub tips: Vec<RenderedTipDto>,
    pub counts: TipsCountsDto,
}

#[derive(Debug, Serialize)]
pub struct RenderedTipDto {
    pub id: String,
    pub category: String,
    pub category_label: String,
    pub prose: String,
    pub prose_b: Option<String>,
    pub experiment_flag: Option<String>,
    pub condition_label: Option<String>,
    pub condition_label_b: Option<String>,
    pub cooldown_sessions: Option<u32>,
    pub last_seen: Option<LastSeenDto>,
    pub trigger_summary: String,
    pub relevance_source: Option<String>,
    pub provider_agnostic: bool,
    pub seen_status: String,
}

#[derive(Debug, Serialize)]
pub struct LastSeenDto {
    pub relative: String,
    pub startup_count_when_seen: u32,
    pub exact_unknown: bool,
}

#[derive(Debug, Serialize)]
pub struct TipsCountsDto {
    pub all: usize,
    pub seen: usize,
    pub never_seen: usize,
    pub active_experiments: usize,
}

#[derive(Debug, Serialize)]
pub struct TipsRefreshDto {
    pub extracted_count: usize,
    pub known_count: usize,
    pub partial: bool,
    pub catalog_version: String,
}

fn category_to_string(c: CoreCategory) -> String {
    // mirror serde rename_all = "kebab-case"
    match c {
        CoreCategory::Onboarding => "onboarding",
        CoreCategory::Workflow => "workflow",
        CoreCategory::Shortcut => "shortcut",
        CoreCategory::Setup => "setup",
        CoreCategory::MemoryConfig => "memory-config",
        CoreCategory::MultiSession => "multi-session",
        CoreCategory::Ide => "ide",
        CoreCategory::AppsExtensions => "apps-extensions",
        CoreCategory::Plugins => "plugins",
        CoreCategory::Experiments => "experiments",
        CoreCategory::Billing => "billing",
        CoreCategory::Misc => "misc",
        CoreCategory::Internal => "internal",
    }
    .to_string()
}

fn seen_to_string(s: CoreSeenStatus) -> String {
    match s {
        CoreSeenStatus::Seen => "seen",
        CoreSeenStatus::NeverSeen => "never-seen",
    }
    .to_string()
}

impl From<CoreLastSeen> for LastSeenDto {
    fn from(s: CoreLastSeen) -> Self {
        Self {
            relative: s.relative,
            startup_count_when_seen: s.startup_count_when_seen,
            exact_unknown: s.exact_unknown,
        }
    }
}

impl From<CoreRenderedTip> for RenderedTipDto {
    fn from(t: CoreRenderedTip) -> Self {
        Self {
            id: t.id,
            category: category_to_string(t.category),
            category_label: t.category_label,
            prose: t.prose,
            prose_b: t.prose_b,
            experiment_flag: t.experiment_flag,
            condition_label: t.condition_label,
            condition_label_b: t.condition_label_b,
            cooldown_sessions: t.cooldown_sessions,
            last_seen: t.last_seen.map(LastSeenDto::from),
            trigger_summary: t.trigger_summary,
            relevance_source: t.relevance_source,
            provider_agnostic: t.provider_agnostic,
            seen_status: seen_to_string(t.seen_status),
        }
    }
}

impl From<CoreTipsCounts> for TipsCountsDto {
    fn from(c: CoreTipsCounts) -> Self {
        Self {
            all: c.all,
            seen: c.seen,
            never_seen: c.never_seen,
            active_experiments: c.active_experiments,
        }
    }
}

impl From<CoreTipsRender> for TipsRenderDto {
    fn from(r: CoreTipsRender) -> Self {
        Self {
            catalog_version: r.catalog_version,
            extracted_at: r.extracted_at,
            partial: r.partial,
            extracted_count: r.extracted_count,
            known_count: r.known_count,
            current_num_startups: r.current_num_startups,
            tips: r.tips.into_iter().map(RenderedTipDto::from).collect(),
            counts: r.counts.into(),
        }
    }
}
