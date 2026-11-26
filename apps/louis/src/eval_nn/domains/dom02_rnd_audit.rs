//! `DOM-02 (rnd_audit)`: National R&D Innovation Act (IRIS / EZbaro / RCMS) & ZEUS 30M KRW equipment audit.

use lv_agent::gates::{select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};

pub const DOM02_FORMS: [&str; 6] = [
    "DOM02_F01_IRIS_AGREEMENT",
    "DOM02_F02_EZBARO_CARD_TX",
    "DOM02_F03_EQUIP_ZEUS_3D",
    "DOM02_F04_RESEARCHER_LABOR",
    "DOM02_F05_MEETING_RECEIPT",
    "DOM02_F06_SPLIT_AUDIT_AUDIO",
];

#[derive(Debug, Clone)]
pub struct Dom02RndAuditInput {
    pub tx_after_agreement_end_unexempt: bool,
    pub restricted_entertainment_merchant: bool,
    pub equip_cost_incl_vat_krw: i64,
    pub equip_pre_approved_zeus: bool,
    pub equip_delivered_2mo_before_end: bool,
    pub equip_gemma4_modality_sim: f32,
    pub total_labor_rate_pct: u16,
    pub gov_institute_special_130pct: bool,
    pub meeting_has_external_attendee: bool,
    pub related_vendor_split_7d: bool,
    pub distinct_homonym_vendor: bool,
    pub audio_payback_detected: bool,
    pub receipt_thermal_margin_missing: bool,
    pub supplement_count: u32,
}

pub fn evaluate_dom02_rnd_audit(
    inp: &Dom02RndAuditInput,
    requested_form: OutputForm,
) -> (AgentVerdict, FormChoice) {
    if inp.equip_gemma4_modality_sim < 0.12 {
        let v = AgentVerdict::G2RejectContradictionLock;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.tx_after_agreement_end_unexempt
        || inp.restricted_entertainment_merchant
        || inp.audio_payback_detected
        || !inp.meeting_has_external_attendee
    {
        let v = AgentVerdict::G3ViolationHighRisk;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.related_vendor_split_7d && !inp.distinct_homonym_vendor {
        let v = AgentVerdict::G3ViolationRelatedVendorSplit;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.equip_cost_incl_vat_krw >= 30_000_000
        && (!inp.equip_pre_approved_zeus || !inp.equip_delivered_2mo_before_end)
    {
        let v = AgentVerdict::G2RejectContradictionLock;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    let labor_cap = if inp.gov_institute_special_130pct { 130 } else { 100 };
    if inp.total_labor_rate_pct > labor_cap {
        let v = AgentVerdict::G2RejectContradictionLock;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.receipt_thermal_margin_missing {
        let v = AgentVerdict::G3HoldSimpleReceiptMissing;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, true));
    }
    let v = if inp.distinct_homonym_vendor {
        AgentVerdict::G3EligibleHomonymClear
    } else {
        AgentVerdict::G3EligibleNormal
    };
    (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dom02_zeus_30m_homonym_and_modality_gates() {
        let base = Dom02RndAuditInput {
            tx_after_agreement_end_unexempt: false,
            restricted_entertainment_merchant: false,
            equip_cost_incl_vat_krw: 32_000_000,
            equip_pre_approved_zeus: true,
            equip_delivered_2mo_before_end: true,
            equip_gemma4_modality_sim: 0.78,
            total_labor_rate_pct: 100,
            gov_institute_special_130pct: false,
            meeting_has_external_attendee: true,
            related_vendor_split_7d: false,
            distinct_homonym_vendor: true,
            audio_payback_detected: false,
            receipt_thermal_margin_missing: false,
            supplement_count: 0,
        };
        assert_eq!(evaluate_dom02_rnd_audit(&base, OutputForm::ApprovalNotice).0, AgentVerdict::G3EligibleHomonymClear);
        assert_eq!(
            evaluate_dom02_rnd_audit(&Dom02RndAuditInput { equip_gemma4_modality_sim: 0.08, ..base.clone() }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G2RejectContradictionLock
        );
        assert_eq!(
            evaluate_dom02_rnd_audit(&Dom02RndAuditInput { related_vendor_split_7d: true, distinct_homonym_vendor: false, ..base }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G3ViolationRelatedVendorSplit
        );
    }
}
