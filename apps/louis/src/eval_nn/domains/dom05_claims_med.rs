//! `DOM-05 (claims_med)`: Gen 1-5 medical indemnity claims (2026 Gen-5 Special 1 vs Special 2), Gemma4 modality gate, Fact-Lock.

use lv_agent::factcheck::draft_or_fallback;
use lv_agent::gates::{select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};
use lv_agent::policy::{AutomationPolicy, Disposition};

pub const DOM05_FORMS: [&str; 6] = [
    "DOM05_F01_CLAIM_APP",
    "DOM05_F02_HOSP_RECEIPT",
    "DOM05_F03_ITEMIZED_DETAIL",
    "DOM05_F04_DIAG_DISCHARGE",
    "DOM05_F05_INJURY_MODALITY",
    "DOM05_F06_PAYOUT_NOTICE_AUDIO",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SilsonGeneration {
    Gen1To4,
    Gen5SevereRider1,    // Annual out-of-pocket cap 5,000,000 KRW
    Gen5NonSevereRider2, // Manual therapy / shockwave / non-benefit injections limited
}

#[derive(Debug, Clone)]
pub struct Dom05ClaimsMedInput {
    pub generation: SilsonGeneration,
    pub injury_photo_gemma4_sim: f32,
    pub receipt_missing: bool,
    pub annual_oop_krw: i64,
    pub cosmetic_excluded_item_krw: i64,
    pub sensitive_kcd_exposed: bool,
    pub draft_notice_text: String,
    pub ledger_reference_text: String,
    pub supplement_count: u32,
}

pub fn evaluate_dom05_claims_med(
    inp: &Dom05ClaimsMedInput,
    requested_form: OutputForm,
    auto_pol: &AutomationPolicy,
) -> (AgentVerdict, FormChoice, Disposition) {
    let (_, fact_ok, _) = draft_or_fallback(
        &inp.draft_notice_text,
        &inp.ledger_reference_text,
        "원장 기준 재확인 안내",
    );
    let verdict = if !fact_ok || inp.sensitive_kcd_exposed {
        AgentVerdict::G4BlockInternalLeak
    } else if inp.injury_photo_gemma4_sim < 0.12 {
        AgentVerdict::G2RejectContradictionLock
    } else if inp.receipt_missing && inp.supplement_count == 0 {
        AgentVerdict::G3HoldSimpleReceiptMissing
    } else if inp.receipt_missing {
        AgentVerdict::G1ExpiredMandatoryReject
    } else if inp.cosmetic_excluded_item_krw > 0
        && matches!(inp.generation, SilsonGeneration::Gen5NonSevereRider2)
    {
        AgentVerdict::G2RejectContradictionLock
    } else {
        AgentVerdict::G3EligibleNormal
    };
    let form = select_form_for_verdict(requested_form, verdict, inp.supplement_count, inp.receipt_missing);
    let disp = auto_pol.decide_disposition(verdict.nature(), Some(0.99), verdict == AgentVerdict::G3EligibleNormal, "DOM05-CLAIM");
    (verdict, form, disp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn dom05_fact_lock_gen5_and_no_adverse_autofinalize() {
        let mut pol = AutomationPolicy::default();
        pol.enable_auto("OFF-MED-1", "법령", Utc::now()).unwrap();
        let bad = Dom05ClaimsMedInput {
            generation: SilsonGeneration::Gen5NonSevereRider2,
            injury_photo_gemma4_sim: 0.85,
            receipt_missing: false,
            annual_oop_krw: 1_200_000,
            cosmetic_excluded_item_krw: 0,
            sensitive_kcd_exposed: false,
            draft_notice_text: "지급액 999,999원".into(),
            ledger_reference_text: "지급액 500,000원".into(),
            supplement_count: 0,
        };
        let (v, _, disp) = evaluate_dom05_claims_med(&bad, OutputForm::ApprovalNotice, &pol);
        assert_eq!(v, AgentVerdict::G4BlockInternalLeak);
        assert!(matches!(disp, Disposition::PendingOfficer { .. }));

        let stock_photo = Dom05ClaimsMedInput {
            injury_photo_gemma4_sim: 0.09,
            draft_notice_text: "지급액 500,000원".into(),
            ..bad
        };
        let (v2, fc2, disp2) = evaluate_dom05_claims_med(&stock_photo, OutputForm::ApprovalNotice, &pol);
        assert_eq!(v2, AgentVerdict::G2RejectContradictionLock);
        assert!(fc2.overridden && fc2.form == OutputForm::RejectNotice);
        assert!(matches!(disp2, Disposition::PendingOfficer { .. }));
    }
}
