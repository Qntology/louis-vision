//! Retail SRM settlement rules (`DOM-10`, Large-Scale Distribution Business Act Art. 11 & 2026 payout deadlines).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetailContractKind {
    DirectPurchaseItemwise,
    DirectPurchaseMonthly,
    SpecialPurchase,
    Consignment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetailSrmCheckInput {
    pub contract_kind: RetailContractKind,
    pub apply_2026_amended_deadline: bool,
    pub base_close_or_receipt_ymd: u32,
    pub scheduled_payout_ymd: u32,
    pub promo_total_cost_krw: i64,
    pub vendor_promo_burden_krw: i64,
    pub promo_written_agreed_ymd: Option<u32>,
    pub promo_start_ymd: Option<u32>,
    pub voluntary_differentiated_proof: bool,
    pub unilateral_return_krw: i64,
    pub return_defect_evidence: bool,
}
