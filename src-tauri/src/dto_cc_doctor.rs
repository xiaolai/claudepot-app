//! DTOs for the `cc_doctor` scrape pipeline.
//!
//! Mirrors the core types but with `camelCase` serde tags so the
//! frontend can consume them without an adapter. Distinct from
//! `dto_service_status` (Claudepot's own health) — this surface
//! carries CC's *own* doctor output.

use claudepot_core::cc_doctor::{
    DoctorSection, DoctorSeverity, DoctorSnapshot, ParseStatus, SectionEntry,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DoctorSeverityDto {
    Healthy,
    Warning,
    Error,
}

impl From<DoctorSeverity> for DoctorSeverityDto {
    fn from(s: DoctorSeverity) -> Self {
        match s {
            DoctorSeverity::Healthy => Self::Healthy,
            DoctorSeverity::Warning => Self::Warning,
            DoctorSeverity::Error => Self::Error,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ParseStatusDto {
    Ok,
    Degraded { reason: String },
    Failed { reason: String },
}

impl From<ParseStatus> for ParseStatusDto {
    fn from(s: ParseStatus) -> Self {
        match s {
            ParseStatus::Ok => Self::Ok,
            ParseStatus::Degraded { reason } => Self::Degraded { reason },
            ParseStatus::Failed { reason } => Self::Failed { reason },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionEntryDto {
    pub text: String,
    pub tree_prefix: String,
}

impl From<SectionEntry> for SectionEntryDto {
    fn from(e: SectionEntry) -> Self {
        Self {
            text: e.text,
            tree_prefix: e.tree_prefix,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorSectionDto {
    pub title: String,
    pub severity: DoctorSeverityDto,
    pub entries: Vec<SectionEntryDto>,
}

impl From<DoctorSection> for DoctorSectionDto {
    fn from(s: DoctorSection) -> Self {
        Self {
            title: s.title,
            severity: s.severity.into(),
            entries: s.entries.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorSnapshotDto {
    pub cc_version: Option<String>,
    pub install_type: Option<String>,
    pub install_path: Option<String>,
    pub severity: DoctorSeverityDto,
    pub sections: Vec<DoctorSectionDto>,
    pub raw_bytes: usize,
    pub parse_status: ParseStatusDto,
    pub captured_at_ms: i64,
}

impl From<DoctorSnapshot> for DoctorSnapshotDto {
    fn from(s: DoctorSnapshot) -> Self {
        Self {
            cc_version: s.cc_version,
            install_type: s.install_type,
            install_path: s.install_path,
            severity: s.severity.into(),
            sections: s.sections.into_iter().map(Into::into).collect(),
            raw_bytes: s.raw_bytes,
            parse_status: s.parse_status.into(),
            captured_at_ms: s.captured_at_ms,
        }
    }
}
