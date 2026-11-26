//! Public procurement G2B costing, financial KYC/governance, tax evidence, and board/single-quote controls (`DOM-11,12,13,18`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostingRuleKind {
    PublicConstructionCost,
    DefenseAcquisitionCost,
    PublicStandardMarketUnit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicG2bCheckInput {
    pub rule_kind: CostingRuleKind,
    pub direct_material_krw: i64,
    pub gov_supplied_material_krw: i64,
    pub subcontract_processing_krw: i64,
    pub tech_royalty_krw: i64,
    pub labor_and_expense_krw: i64,
    pub general_admin_claimed_krw: i64,
    pub general_admin_cap_bps: u32,
    pub profit_claimed_krw: i64,
    pub profit_cap_bps: u32,
    pub missing_rfp_mandatory_items: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinKycLoanCheckInput {
    pub accountability_code: Option<String>,
    pub control_action_logged: bool,
    pub ubo_25pct_natural_person_verified: bool,
    pub fallback_controlling_person_verified: bool,
    pub registry_vs_shareholder_sum_match: bool,
    pub high_risk_edd_triggered: bool,
    pub edd_enhanced_docs_attached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaxEvidCheckInput {
    pub etax_issue_ymd: u32,
    pub etax_nts_transmit_ymd: Option<u32>,
    pub promo_expense_krw: i64,
    pub vat_input_credit_claimed: bool,
    pub is_traditional_market_or_local_voucher: bool,
    pub is_entertainment_excluded_venue: bool,
    pub is_condolence_expense: bool,
    pub paid_by_corporate_qualified_proof: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditGovCheckInput {
    pub single_quote_contract_krw: i64,
    pub is_women_disabled_youth_cert: bool,
    pub is_sme_restricted_bid_but_mid_corp: bool,
    pub same_vendor_annual_count: u32,
    pub same_vendor_annual_sum_krw: i64,
    pub annual_count_cap: u32,
    pub annual_sum_cap_krw: i64,
    pub board_approved_cap_krw: Option<i64>,
    pub cumulative_executed_krw: i64,
}
