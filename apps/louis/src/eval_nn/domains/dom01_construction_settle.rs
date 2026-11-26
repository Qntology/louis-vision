//! `DOM-01 (construction_settle)`: Construction / plant / temporary stage progress settlement & labor segregation.

use lv_agent::gates::{check_citizen_text, select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};

use super::contains_forbidden_raw_gov_id_digits;

pub const DOM01_FORMS: [&str; 6] = [
    "DOM01_F01_SUBCONTRACT",
    "DOM01_F02_PROGRESS_CLAIM",
    "DOM01_F03_BOQ_DETAIL",
    "DOM01_F04_LABOR_SEPARATE",
    "DOM01_F05_E_BILL_GUARANTEE",
    "DOM01_F06_TEMP_STAGE_PE",
];

#[derive(Debug, Clone)]
pub struct Dom01ConstructionInput {
    pub contract_sum_krw: i64,
    pub progress_bps: u32,
    pub prev_paid_krw: i64,
    pub advance_deduct_krw: i64,
    pub claimed_amount_krw: i64,
    pub pay_guarantee_or_direct_agree: bool,
    pub separate_labor_account_ok: bool,
    pub e_bill_over_60d_unpaid_discount: bool,
    pub del_unit_price_tamper: bool,
    pub pe_seal_verified: bool,
    pub center_lift_payload_kg: u32,
    pub center_lift_stroke_mm: u32,
    pub draft_notice_text: String,
    pub supplement_count: u32,
}

pub fn evaluate_dom01_construction_settle(
    inp: &Dom01ConstructionInput,
    requested_form: OutputForm,
) -> (AgentVerdict, FormChoice) {
    if contains_forbidden_raw_gov_id_digits(&inp.draft_notice_text)
        || check_citizen_text(&inp.draft_notice_text, &[]).is_err()
    {
        let v = AgentVerdict::G4BlockInternalLeak;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if !inp.separate_labor_account_ok || !inp.pay_guarantee_or_direct_agree || inp.e_bill_over_60d_unpaid_discount {
        let v = AgentVerdict::G3ViolationHighRisk;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    let ceiling = ((inp.contract_sum_krw as i128 * inp.progress_bps as i128 / 10_000) as i64)
        - inp.prev_paid_krw
        - inp.advance_deduct_krw;
    if inp.del_unit_price_tamper || inp.claimed_amount_krw > ceiling || !inp.pe_seal_verified {
        let v = AgentVerdict::G2RejectContradictionLock;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.center_lift_payload_kg > 500 || inp.center_lift_stroke_mm > 2_500 {
        let v = AgentVerdict::G3ViolationHighRisk;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    let v = AgentVerdict::G3EligibleNormal;
    (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dom01_progress_formula_and_citizen_term_block() {
        let ok = Dom01ConstructionInput {
            contract_sum_krw: 500_000_000,
            progress_bps: 3_000, // 30% -> 150,000,000
            prev_paid_krw: 40_000_000,
            advance_deduct_krw: 15_000_000,
            claimed_amount_krw: 95_000_000,
            pay_guarantee_or_direct_agree: true,
            separate_labor_account_ok: true,
            e_bill_over_60d_unpaid_discount: false,
            del_unit_price_tamper: false,
            pe_seal_verified: true,
            center_lift_payload_kg: 500,
            center_lift_stroke_mm: 2_500,
            draft_notice_text: "일용근로자 [RRN_REDACTED] 기성금 95,000,000원 지급 적합".into(),
            supplement_count: 0,
        };
        assert_eq!(evaluate_dom01_construction_settle(&ok, OutputForm::ApprovalNotice).0, AgentVerdict::G3EligibleNormal);
        let over = Dom01ConstructionInput { claimed_amount_krw: 95_000_001, ..ok.clone() };
        let (v, fc) = evaluate_dom01_construction_settle(&over, OutputForm::ApprovalNotice);
        assert_eq!(v, AgentVerdict::G2RejectContradictionLock);
        assert!(fc.overridden && fc.form == OutputForm::RejectNotice);

        let leak = Dom01ConstructionInput {
            draft_notice_text: "위장하도급 검토 요망 LatticeDB".into(),
            ..ok
        };
        assert_eq!(evaluate_dom01_construction_settle(&leak, OutputForm::ApprovalNotice).0, AgentVerdict::G4BlockInternalLeak);
    }
}
