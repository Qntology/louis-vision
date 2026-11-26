//! `DOM-11 (public_g2b_contract)`: Public procurement costing (Art. 41 Std Market vs general) & 2026 Defense Costing (MinDef Order 1202).

use lv_agent::gates::{select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};
use lv_doc::writer::guard::N2sfGrade;

pub const DOM11_FORMS: [&str; 6] = [
    "DOM11_F01_RFP_SPEC",
    "DOM11_F02_COST_SUMMARY",
    "DOM11_F03_DEFENSE_COST",
    "DOM11_F04_UNIT_QUOTE",
    "DOM11_F05_GFM_LEDGER",
    "DOM11_F06_COST_REVIEW",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicCostRegime2026 {
    GeneralConstruction,
    StdMarketPriceArt41,
    GoodsManufacturing,
    AcademicResearchService,
    DefenseCostDapa2026,
}

#[derive(Debug, Clone)]
pub struct Dom11PublicG2bCostInput {
    pub regime: PublicCostRegime2026,
    pub doc_n2sf_grade: N2sfGrade,
    pub target_zone_grade: N2sfGrade,
    pub covert_artifact_or_del_count: u32,
    pub declassification_ref_present: bool,
    pub net_material_krw: i64,
    pub gfm_illegally_in_net_cost: bool,
    pub total_labor_krw: i64,
    pub total_expense_krw: i64,
    pub outsource_and_tech_fee_krw: i64,
    pub defense_non_cost_included_krw: i64,
    pub defense_risk_base_includes_mat: bool,
    pub defense_effort_base_includes_import: bool,
    pub gen_admin_rate_bps: u32,
    pub gen_admin_cap_bps: u32,
    pub gen_admin_claimed_krw: i64,
    pub profit_rate_bps: u32,
    pub profit_claimed_krw: i64,
    pub detail_boq_sum_matches_header: bool,
    pub missing_price_quote_evidence: bool,
    pub audio_reserve_price_leak_or_rigging: bool,
    pub supplement_count: u32,
}

pub fn legal_max_profit_krw(inp: &Dom11PublicG2bCostInput, expected_admin_krw: i64) -> (u32, i64) {
    match inp.regime {
        PublicCostRegime2026::GeneralConstruction => {
            let cap_bps = 1_500u32;
            let exp = (inp.total_expense_krw - inp.outsource_and_tech_fee_krw).max(0);
            let base = inp.total_labor_krw + exp + expected_admin_krw;
            (cap_bps, (base as i128 * inp.profit_rate_bps.min(cap_bps) as i128 / 10_000) as i64)
        }
        PublicCostRegime2026::StdMarketPriceArt41 => {
            let cap_bps = 1_500u32;
            let base = inp.net_material_krw + inp.total_labor_krw + inp.total_expense_krw + expected_admin_krw;
            (cap_bps, (base as i128 * inp.profit_rate_bps.min(cap_bps) as i128 / 10_000) as i64)
        }
        PublicCostRegime2026::GoodsManufacturing => {
            let cap_bps = 2_500u32;
            let exp = (inp.total_expense_krw - inp.outsource_and_tech_fee_krw).max(0);
            let base = inp.total_labor_krw + exp + expected_admin_krw;
            (cap_bps, (base as i128 * inp.profit_rate_bps.min(cap_bps) as i128 / 10_000) as i64)
        }
        PublicCostRegime2026::AcademicResearchService => {
            let cap_bps = 1_000u32;
            let base = inp.total_labor_krw + inp.total_expense_krw + expected_admin_krw;
            (cap_bps, (base as i128 * inp.profit_rate_bps.min(cap_bps) as i128 / 10_000) as i64)
        }
        PublicCostRegime2026::DefenseCostDapa2026 => (2_500u32, inp.profit_claimed_krw),
    }
}

pub fn evaluate_dom11_public_g2b_cost(
    inp: &Dom11PublicG2bCostInput,
    requested_form: OutputForm,
) -> (AgentVerdict, FormChoice) {
    if (inp.doc_n2sf_grade == N2sfGrade::Classified && inp.target_zone_grade == N2sfGrade::Open)
        || inp.audio_reserve_price_leak_or_rigging
    {
        let v = AgentVerdict::G3ViolationHighRisk;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.doc_n2sf_grade == N2sfGrade::Sensitive
        && inp.target_zone_grade == N2sfGrade::Open
        && (!inp.declassification_ref_present || inp.covert_artifact_or_del_count > 0)
    {
        let v = if inp.covert_artifact_or_del_count > 0 {
            AgentVerdict::G3ViolationHighRisk
        } else {
            AgentVerdict::G1InterceptForceHold
        };
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.missing_price_quote_evidence {
        let v = if inp.supplement_count == 0 {
            AgentVerdict::G3HoldSimpleReceiptMissing
        } else {
            AgentVerdict::G1ExpiredMandatoryReject
        };
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, true));
    }
    let net_cost = inp.net_material_krw + inp.total_labor_krw + inp.total_expense_krw;
    let expected_admin = (net_cost as i128 * inp.gen_admin_rate_bps as i128 / 10_000) as i64;
    let (profit_cap, allowed_profit) = legal_max_profit_krw(inp, expected_admin);
    let defense_viol = matches!(inp.regime, PublicCostRegime2026::DefenseCostDapa2026)
        && (inp.defense_non_cost_included_krw > 0
            || inp.defense_risk_base_includes_mat
            || inp.defense_effort_base_includes_import);

    if inp.covert_artifact_or_del_count > 0
        || inp.gfm_illegally_in_net_cost
        || !inp.detail_boq_sum_matches_header
        || inp.gen_admin_rate_bps > inp.gen_admin_cap_bps
        || inp.gen_admin_claimed_krw > expected_admin
        || inp.profit_rate_bps > profit_cap
        || inp.profit_claimed_krw > allowed_profit
        || defense_viol
    {
        let v = AgentVerdict::G2RejectContradictionLock;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    let v = AgentVerdict::G3EligibleNormal;
    (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dom11_art41_vs_general_profit_and_defense_rules() {
        let art41 = Dom11PublicG2bCostInput {
            regime: PublicCostRegime2026::StdMarketPriceArt41,
            doc_n2sf_grade: N2sfGrade::Sensitive,
            target_zone_grade: N2sfGrade::Sensitive,
            covert_artifact_or_del_count: 0,
            declassification_ref_present: true,
            net_material_krw: 400_000_000,
            gfm_illegally_in_net_cost: false,
            total_labor_krw: 500_000_000,
            total_expense_krw: 100_000_000,
            outsource_and_tech_fee_krw: 50_000_000,
            defense_non_cost_included_krw: 0,
            defense_risk_base_includes_mat: false,
            defense_effort_base_includes_import: false,
            gen_admin_rate_bps: 800,
            gen_admin_cap_bps: 800,
            gen_admin_claimed_krw: 80_000_000,
            profit_rate_bps: 1_500,
            profit_claimed_krw: 162_000_000, // 15% * (1,000M + 80M) = 162M lawful under Art. 41!
            detail_boq_sum_matches_header: true,
            missing_price_quote_evidence: false,
            audio_reserve_price_leak_or_rigging: false,
            supplement_count: 0,
        };
        assert_eq!(evaluate_dom11_public_g2b_cost(&art41, OutputForm::ApprovalNotice).0, AgentVerdict::G3EligibleNormal);
        let general_illegal = Dom11PublicG2bCostInput { regime: PublicCostRegime2026::GeneralConstruction, ..art41.clone() };
        assert_eq!(evaluate_dom11_public_g2b_cost(&general_illegal, OutputForm::ApprovalNotice).0, AgentVerdict::G2RejectContradictionLock);
        let c2o = Dom11PublicG2bCostInput { doc_n2sf_grade: N2sfGrade::Classified, target_zone_grade: N2sfGrade::Open, ..art41 };
        assert_eq!(evaluate_dom11_public_g2b_cost(&c2o, OutputForm::ApprovalNotice).0, AgentVerdict::G3ViolationHighRisk);
    }
}
