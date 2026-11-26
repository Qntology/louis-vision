//! `DOM-04 (finance_3way)`: Corporate PO <-> GR <-> Invoice 3-way matching, master bank lock, and manual upward cap.

use lv_agent::gates::{check_manual_amount, select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};

pub const DOM04_FORMS: [&str; 6] = [
    "DOM04_F01_PURCHASE_ORDER",
    "DOM04_F02_GOODS_RECEIPT",
    "DOM04_F03_AP_INVOICE",
    "DOM04_F04_VENDOR_MASTER_CHG",
    "DOM04_F05_DISBURSE_VOUCHER",
    "DOM04_F06_PROCURE_AUDIO",
];

#[derive(Debug, Clone)]
pub struct Dom04Finance3WayInput {
    pub po_qty: u32,
    pub gr_qty: u32,
    pub invoice_qty: u32,
    pub master_account_hash: String,
    pub submitted_account_hash: String,
    pub homoglyph_account_detected: bool,
    pub unit_label: String,
    pub split_3times_in_7d: bool,
    pub verified_sum_krw: i64,
    pub requested_disburse_krw: i64,
    pub evidence_headroom_krw: i64,
    pub audio_rebate_collusion: bool,
    pub supplement_count: u32,
}

pub fn evaluate_dom04_finance_3way(
    inp: &Dom04Finance3WayInput,
    requested_form: OutputForm,
) -> (AgentVerdict, FormChoice) {
    if inp.homoglyph_account_detected {
        let v = AgentVerdict::G4BlockInternalLeak;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.master_account_hash != inp.submitted_account_hash || inp.audio_rebate_collusion {
        let v = AgentVerdict::G3ViolationHighRisk;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.split_3times_in_7d {
        let v = AgentVerdict::G3ViolationRelatedVendorSplit;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if let Err(v) = check_manual_amount(
        inp.verified_sum_krw,
        inp.requested_disburse_krw,
        inp.evidence_headroom_krw,
    ) {
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.invoice_qty > inp.gr_qty || inp.gr_qty > inp.po_qty || inp.unit_label.trim().is_empty() {
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
    fn dom04_3way_match_and_unproven_upward_block() {
        let ok = Dom04Finance3WayInput {
            po_qty: 100,
            gr_qty: 100,
            invoice_qty: 100,
            master_account_hash: "0xabc123".into(),
            submitted_account_hash: "0xabc123".into(),
            homoglyph_account_detected: false,
            unit_label: "PCS".into(),
            split_3times_in_7d: false,
            verified_sum_krw: 80_000_000,
            requested_disburse_krw: 80_000_000,
            evidence_headroom_krw: 0,
            audio_rebate_collusion: false,
            supplement_count: 0,
        };
        assert_eq!(evaluate_dom04_finance_3way(&ok, OutputForm::ApprovalNotice).0, AgentVerdict::G3EligibleNormal);
        assert_eq!(
            evaluate_dom04_finance_3way(&Dom04Finance3WayInput { requested_disburse_krw: 80_000_001, ..ok.clone() }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G2BlockUnprovenEdit
        );
        assert_eq!(
            evaluate_dom04_finance_3way(&Dom04Finance3WayInput { homoglyph_account_detected: true, ..ok }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G4BlockInternalLeak
        );
    }
}
