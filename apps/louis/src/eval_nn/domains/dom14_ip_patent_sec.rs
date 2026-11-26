//! `DOM-14 (ip_patent_sec)`: 79 National Core Tech (30d registration, export vs prior-filing),
//! automatic employee-invention succession (0 false rejects), Art. 41 defense patent block, `veryHidden` Classified escalation,
//! N2SF `C -> O` 0B block, `S -> O Reverse-CDR`, and zero-digit government IDs.

use lv_agent::gates::{select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};
use lv_doc::writer::guard::{N2sfGrade, ReverseCdrContext, WriterGuard};

use super::contains_forbidden_raw_gov_id_digits;

pub const DOM14_FORMS: [&str; 6] = [
    "DOM14_F01_NCT_PRECHECK",
    "DOM14_F02_MOTIE_EXPORT",
    "DOM14_F03_INVENTION_DECL",
    "DOM14_F04_SPEC_DRAWING",
    "DOM14_F05_N2SF_CDR_REQ",
    "DOM14_F06_GATE_3D_AUDIO",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TechFundingRoute {
    GovFundedRequiresPriorApproval,
    SelfFundedRequiresPreFilingReceipt,
}

#[derive(Debug, Clone)]
pub struct Dom14IpPatentSecInput {
    pub is_nct_79: bool,
    pub days_since_nct_determination: i32,
    pub registered_with_motie: bool,
    pub funding_route: TechFundingRoute,
    pub motie_approval_or_receipt_present: bool,
    pub has_reservation_succession_rule: bool,
    pub days_since_invention_completion: i32,
    pub explicit_non_succession_notice: bool,
    pub is_defense_patent_art41: bool,
    pub foreign_pct_without_clearance: bool,
    pub xlsx_has_very_hidden_sheet: bool,
    pub nominal_doc_grade: N2sfGrade,
    pub target_zone_grade: N2sfGrade,
    pub declassification_ref: Option<String>,
    pub ciso_0x_cosigned: bool,
    pub covert_artifact_count: usize,
    pub external_beacon_present: bool,
    pub audio_gas_flow_leak_detected: bool,
    pub body_text: String,
    pub supplement_count: u32,
}

pub fn effective_n2sf_grade(inp: &Dom14IpPatentSecInput) -> N2sfGrade {
    if inp.xlsx_has_very_hidden_sheet {
        N2sfGrade::Classified
    } else {
        inp.nominal_doc_grade
    }
}

pub fn evaluate_dom14_ip_patent_sec(
    inp: &Dom14IpPatentSecInput,
    requested_form: OutputForm,
) -> (AgentVerdict, FormChoice, Vec<u8>) {
    if contains_forbidden_raw_gov_id_digits(&inp.body_text) {
        let v = AgentVerdict::G4BlockInternalLeak;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false), Vec::new());
    }
    let eff_grade = effective_n2sf_grade(inp);
    if (eff_grade == N2sfGrade::Classified && inp.target_zone_grade == N2sfGrade::Open)
        || inp.foreign_pct_without_clearance && inp.is_defense_patent_art41
        || inp.external_beacon_present
        || inp.covert_artifact_count > 0
        || inp.audio_gas_flow_leak_detected
    {
        let v = AgentVerdict::G3ViolationHighRisk;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false), Vec::new());
    }
    if eff_grade == N2sfGrade::Sensitive
        && inp.target_zone_grade == N2sfGrade::Open
        && (inp.declassification_ref.as_deref().is_none_or(|s| s.trim().is_empty()) || !inp.ciso_0x_cosigned)
    {
        let v = AgentVerdict::G1InterceptForceHold;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false), Vec::new());
    }
    if inp.is_nct_79 && !inp.registered_with_motie && inp.days_since_nct_determination > 30 {
        let v = AgentVerdict::G1ExpiredMandatoryReject;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false), Vec::new());
    }
    if !inp.has_reservation_succession_rule
        && !inp.explicit_non_succession_notice
        && inp.days_since_invention_completion > 120
    {
        let v = AgentVerdict::G1ExpiredMandatoryReject;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false), Vec::new());
    }
    if !inp.motie_approval_or_receipt_present {
        let v = match inp.funding_route {
            TechFundingRoute::GovFundedRequiresPriorApproval => AgentVerdict::G3ViolationHighRisk,
            TechFundingRoute::SelfFundedRequiresPreFilingReceipt if inp.supplement_count == 0 => {
                AgentVerdict::G3HoldSimpleReceiptMissing
            }
            TechFundingRoute::SelfFundedRequiresPreFilingReceipt => AgentVerdict::G1ExpiredMandatoryReject,
        };
        let simple = matches!(v, AgentVerdict::G3HoldSimpleReceiptMissing);
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, simple), Vec::new());
    }

    let guard = WriterGuard {
        forbidden: Vec::new(),
        locked_facts: Vec::new(),
        enforce_internal_terms: true,
        reverse_cdr: Some(ReverseCdrContext {
            source_grade: Some(eff_grade),
            target_grade: Some(inp.target_zone_grade),
            declassification_ref: inp.declassification_ref.clone(),
            stripped_covert_artifacts: inp.covert_artifact_count,
            block_on_covert_artifacts: true,
        }),
    };
    if guard.verify_text(&inp.body_text).is_err() {
        let v = AgentVerdict::G4BlockInternalLeak;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false), Vec::new());
    }
    let v = AgentVerdict::G3EligibleNormal;
    (
        v,
        select_form_for_verdict(requested_form, v, inp.supplement_count, false),
        inp.body_text.as_bytes().to_vec(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dom14_very_hidden_c2o_0b_auto_succession_and_zero_digit_ids() {
        let base = Dom14IpPatentSecInput {
            is_nct_79: true,
            days_since_nct_determination: 20,
            registered_with_motie: true,
            funding_route: TechFundingRoute::SelfFundedRequiresPreFilingReceipt,
            motie_approval_or_receipt_present: true,
            has_reservation_succession_rule: true, // Automatic succession -> 0 false rejects
            days_since_invention_completion: 150,
            explicit_non_succession_notice: false,
            is_defense_patent_art41: false,
            foreign_pct_without_clearance: false,
            xlsx_has_very_hidden_sheet: false,
            nominal_doc_grade: N2sfGrade::Sensitive,
            target_zone_grade: N2sfGrade::Open,
            declassification_ref: Some("N2SF-DECL-2026-01".into()),
            ciso_0x_cosigned: true,
            covert_artifact_count: 0,
            external_beacon_present: false,
            audio_gas_flow_leak_detected: false,
            body_text: "공동발명자 [RRN_REDACTED] / [MYNUMBER_REDACTED] / [AADHAAR_REDACTED] 역CDR 승인본".into(),
            supplement_count: 0,
        };
        let (v_ok, _, emitted) = evaluate_dom14_ip_patent_sec(&base, OutputForm::ApprovalNotice);
        assert_eq!(v_ok, AgentVerdict::G3EligibleNormal);
        assert!(!emitted.is_empty());

        let hidden = Dom14IpPatentSecInput { xlsx_has_very_hidden_sheet: true, ..base.clone() };
        let (v_c2o, _, emitted_c2o) = evaluate_dom14_ip_patent_sec(&hidden, OutputForm::ApprovalNotice);
        assert_eq!(v_c2o, AgentVerdict::G3ViolationHighRisk);
        assert_eq!(emitted_c2o.len(), 0);

        let overdue_reg = Dom14IpPatentSecInput { registered_with_motie: false, days_since_nct_determination: 31, ..base };
        assert_eq!(evaluate_dom14_ip_patent_sec(&overdue_reg, OutputForm::ApprovalNotice).0, AgentVerdict::G1ExpiredMandatoryReject);
    }
}
