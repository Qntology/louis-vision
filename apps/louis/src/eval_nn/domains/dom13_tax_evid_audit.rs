//! `DOM-13 (tax_evid_audit)`: 24-digit NTS XML e-Tax Invoice, biz-reg monetary axis collision bounce,
//! 2026 Onnuri/local-market 20% promotion cap, and Fact-Lock.

use lv_agent::factcheck::draft_or_fallback;
use lv_agent::gates::{check_citizen_text, select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};

use super::super::track_c_gov_fin_tax::next_month_10th_ymd;

pub const DOM13_FORMS: [&str; 6] = [
    "DOM13_F01_NTS_ETAX_XML",
    "DOM13_F02_CARD_EXP_LEDGER",
    "DOM13_F03_PROMO_ADJ_FORM",
    "DOM13_F04_RECEIPT_MKT_3D",
    "DOM13_F05_TAX_EXPLAIN_LTR",
    "DOM13_F06_TAX_AUDIT_AUDIO",
];

pub fn verify_etax_approval_no_24(raw: &str) -> Option<String> {
    let s: String = raw.chars().filter(|c| *c != '-' && !c.is_whitespace()).collect();
    if s.len() != 24 {
        return None;
    }
    let (ymd8, rest) = s.split_at(8);
    let (cert8, serial8) = rest.split_at(8);
    if !ymd8.chars().all(|c| c.is_ascii_digit())
        || !cert8.chars().all(|c| c.is_ascii_digit())
        || !matches!(&cert8[..2], "10" | "20" | "41" | "42" | "50" | "70" | "90")
        || !serial8.chars().all(|c| c.is_ascii_digit() || c.is_ascii_lowercase())
    {
        return None;
    }
    Some(s)
}

/// Bounces OCR scan misalignments where a 10-digit BizRegNo collides into supply or tax monetary slots without LLM calls.
pub fn reconcile_tax_monetary_axes(
    supply_cand: Option<i64>,
    tax_cand: Option<i64>,
    grand_total: Option<i64>,
    line_sum_supply: Option<i64>,
    known_biz_nos_10: &[&str],
) -> (Option<i64>, Option<i64>, bool) {
    let mut bounced = false;
    let is_biz_collision = |val: i64| -> bool {
        let s = val.to_string();
        s.len() == 10 && known_biz_nos_10.iter().any(|b| b.replace('-', "") == s)
    };
    let mut sup = supply_cand.filter(|&v| {
        if v < 0
            || is_biz_collision(v)
            || line_sum_supply.filter(|&ls| ls > 0).is_some_and(|ls| v > ls.saturating_mul(10))
        {
            bounced = true;
            return false;
        }
        true
    });
    let tax = tax_cand.filter(|&t| {
        if t < 0 || is_biz_collision(t) || sup.is_some_and(|s| t > s) {
            bounced = true;
            return false;
        }
        true
    });
    if sup.is_none() {
        if let (Some(gt), Some(t)) = (grand_total, tax) {
            if gt >= t {
                sup = Some(gt - t);
            }
        }
    }
    (sup, tax, bounced)
}

#[derive(Debug, Clone)]
pub struct Dom13TaxEvidAuditInput {
    pub etax_approval_no: String,
    pub supply_ymd: u32,
    pub issued_ymd: u32,
    pub nts_transmitted_ymd: Option<u32>,
    pub promo_expense_krw: i64,
    pub is_condolence_expense: bool,
    pub personal_card_used: bool,
    pub vat_input_credit_claimed: bool,
    pub is_traditional_market_or_onnuri: bool,
    pub is_entertainment_excluded_venue: bool,
    pub extra_20pct_cap_claimed: bool,
    pub explain_letter_draft: String,
    pub ledger_reference_text: String,
    pub supplement_count: u32,
}

pub fn evaluate_dom13_tax_evid_audit(
    inp: &Dom13TaxEvidAuditInput,
    requested_form: OutputForm,
) -> (AgentVerdict, FormChoice) {
    let (_, fact_ok, _) = draft_or_fallback(
        &inp.explain_letter_draft,
        &inp.ledger_reference_text,
        "원장 기준 과세자료 재안내",
    );
    if !fact_ok || check_citizen_text(&inp.explain_letter_draft, &[]).is_err() {
        let v = AgentVerdict::G4BlockInternalLeak;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if verify_etax_approval_no_24(&inp.etax_approval_no).is_none() {
        let v = AgentVerdict::G2RejectContradictionLock;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    let issue_deadline = next_month_10th_ymd(inp.supply_ymd);
    if inp.issued_ymd > issue_deadline || inp.nts_transmitted_ymd.is_none_or(|tx| tx > issue_deadline) {
        let v = AgentVerdict::G1ExpiredMandatoryReject;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    let proof_cap = if inp.is_condolence_expense { 200_000 } else { 30_000 };
    if inp.vat_input_credit_claimed
        || (inp.promo_expense_krw > proof_cap && inp.personal_card_used)
        || (inp.extra_20pct_cap_claimed
            && (!inp.is_traditional_market_or_onnuri || inp.is_entertainment_excluded_venue))
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
    fn dom13_reconcile_biz_reg_collision_personal_card_and_onnuri_20pct() {
        let (sup, tax, bounced) = reconcile_tax_monetary_axes(
            Some(1270221736),
            Some(15_000),
            Some(165_000),
            Some(150_000),
            &["127-02-21736"],
        );
        assert!(bounced);
        assert_eq!(sup, Some(150_000));
        assert_eq!(tax, Some(15_000));

        let ok = Dom13TaxEvidAuditInput {
            etax_approval_no: "20261031-41000012-a9b8c7d6".into(),
            supply_ymd: 2026_1020,
            issued_ymd: 2026_1110,
            nts_transmitted_ymd: Some(2026_1110),
            promo_expense_krw: 150_000,
            is_condolence_expense: false,
            personal_card_used: false,
            vat_input_credit_claimed: false,
            is_traditional_market_or_onnuri: true,
            is_entertainment_excluded_venue: false,
            extra_20pct_cap_claimed: true,
            explain_letter_draft: "공급가액 150,000원 확인".into(),
            ledger_reference_text: "원장 공급가액 150,000원".into(),
            supplement_count: 0,
        };
        assert_eq!(evaluate_dom13_tax_evid_audit(&ok, OutputForm::ApprovalNotice).0, AgentVerdict::G3EligibleNormal);
        assert_eq!(
            evaluate_dom13_tax_evid_audit(&Dom13TaxEvidAuditInput { is_entertainment_excluded_venue: true, ..ok.clone() }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G2RejectContradictionLock
        );
        assert_eq!(
            evaluate_dom13_tax_evid_audit(&Dom13TaxEvidAuditInput { personal_card_used: true, ..ok }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G2RejectContradictionLock
        );
    }
}
