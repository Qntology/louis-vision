//! WORM audit retention classifications (`1,825d / 1,095d / 730d / 365d`) and 11-field paradata schema.

use lv_agent::gates::AgentVerdict;
use lv_agent::policy::Disposition;
use lv_doc::writer::guard::N2sfGrade;
use lv_domain::EngineMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionClass {
    HighImpactAi5Y,
    PrivacyPermission3Y,
    PrivacyAccessSensitive2Y,
    PrivacyAccessStandard1Y,
}

impl RetentionClass {
    pub const fn min_retention_days(self) -> u32 {
        match self {
            Self::HighImpactAi5Y => 1_825,
            Self::PrivacyPermission3Y => 1_095,
            Self::PrivacyAccessSensitive2Y => 730,
            Self::PrivacyAccessStandard1Y => 365,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SealedAuditPayload {
    pub timestamp_utc: String,
    pub case_id: String,
    pub manifest_rev: String,
    pub docs_defs_rev: String,
    pub engine_mode: EngineMode,
    pub n2sf_grade: N2sfGrade,
    pub verdict: AgentVerdict,
    pub confidence: Option<f32>,
    pub disposition: Disposition,
    pub officer_id: Option<String>,
    pub justification: Option<String>,
    pub retention_class: RetentionClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_hmac_tag: Option<String>,
}
