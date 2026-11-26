//! Core tech IP security (N2SF), ECN/BOM engineering change, defense 3D MRO, and HR blind privacy (`DOM-14..17`).

use lv_sds::N2sfGrade;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpPatentSecCheckInput {
    pub hidden_prompt_or_zero_pt_text_found: bool,
    pub external_beacon_url_found: bool,
    pub detected_grade: N2sfGrade,
    pub target_zone_grade: N2sfGrade,
    pub is_national_core_tech_79: bool,
    pub motie_export_or_prior_filing_no: Option<String>,
    pub security_officer_cosigned: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngEcnBomCheckInput {
    pub title_block_recalled_count: u32,
    pub ecn_or_moc_no: Option<String>,
    pub ecn_status_approved: bool,
    pub scanned_drawing_is_latest_rev: bool,
    pub signer_sector_authorized: bool,
    pub proposed_param_value: f64,
    pub design_safety_limit: f64,
    pub drawing_symbol_tag_count: u32,
    pub bom_active_line_qty_sum: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DefenseMroCheckInput {
    pub interlock_out_of_order_attempted: bool,
    pub plaque_effective_px_per_mm: f32,
    pub scanned_part_no_matches_baseline: bool,
    pub approved_ecp_or_waiver_no: Option<String>,
    pub coc_certificate_verified: bool,
    pub measured_torque_within_spec: bool,
    pub safety_interlock_complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiringAuditProfile {
    Standard,
    PublicBlindStrict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HrPrivacyCheckInput {
    pub art4_3_prohibited_field_count: u32,
    pub career_claimed_days: u32,
    pub career_cert_days_verified: u32,
    pub profile: HiringAuditProfile,
    pub blind_school_age_gender_leak_count: u32,
    pub is_disciplinary_or_rejection_action: bool,
}
