//! `DOM-17 (hr_comp_privacy)`: Hiring Procedure Act Art. 4-3 vs Public Blind, day-level overlapping career interval union,
//! 2026 AI Basic Act Art. 34 human-in-the-loop (`AutoFinalize == 0`), and EpochWatch / Kiosk lock.

use std::collections::BTreeMap;

use lv_agent::gates::{select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};
use lv_agent::policy::{AutomationPolicy, Disposition};
use lv_domain::sec_eng_mro_hr::HiringAuditProfile;
use lv_router::vault::{DeviceRole, VaultError};

pub const DOM17_FORMS: [&str; 6] = [
    "DOM17_F01_JOB_NOTICE_APP",
    "DOM17_F02_NCS_ESSAY",
    "DOM17_F03_CAREER_NHIS",
    "DOM17_F04_EQUITY_PROOF",
    "DOM17_F05_INTERVIEW_PACKET",
    "DOM17_F06_INTERVIEW_AUDIO",
];

/// Merges overlapping employment intervals `(start_day_idx, end_day_exclusive, fte_ratio_0_to_1)` on a per-day grid,
/// capping concurrent jobs on the same calendar day at `1.0` full-time equivalent day.
pub fn compute_verified_career_days_union(intervals: &[(i32, i32, f32)]) -> u32 {
    let mut day_fte: BTreeMap<i32, f32> = BTreeMap::new();
    for &(start, end_excl, fte) in intervals {
        let clamped = fte.clamp(0.0, 1.0);
        for d in start..end_excl {
            let entry = day_fte.entry(d).or_insert(0.0);
            *entry = (*entry + clamped).min(1.0);
        }
    }
    day_fte.values().copied().sum::<f32>().round() as u32
}

#[derive(Debug, Clone)]
pub struct Dom17BlindHrRecruitInput {
    pub terminal_role: DeviceRole,
    pub profile: HiringAuditProfile,
    pub art4_3_prohibited_count: u32,
    pub blind_school_leak_in_essay: bool,
    pub career_claimed_days: u32,
    pub career_intervals: Vec<(i32, i32, f32)>,
    pub is_adverse_or_disciplinary: bool,
    pub art11_doc_return_delay_days: u32,
    pub epoch_privacy_locked: bool,
    pub audio_prohibited_family_question: bool,
    pub supplement_count: u32,
}

pub fn evaluate_dom17_blind_hr(
    inp: &Dom17BlindHrRecruitInput,
    requested_form: OutputForm,
    auto_pol: &AutomationPolicy,
) -> Result<(AgentVerdict, FormChoice, Disposition), VaultError> {
    if matches!(inp.terminal_role, DeviceRole::CounterKiosk) {
        return Err(VaultError::KioskForbidden);
    }
    let verified_days = compute_verified_career_days_union(&inp.career_intervals);
    let verdict = if inp.epoch_privacy_locked {
        AgentVerdict::G4BlockInternalLeak
    } else if inp.art4_3_prohibited_count > 0 || inp.audio_prohibited_family_question {
        AgentVerdict::G3ViolationHighRisk
    } else if inp.career_claimed_days > verified_days {
        AgentVerdict::G2RejectContradictionLock
    } else if matches!(inp.profile, HiringAuditProfile::PublicBlindStrict)
        && inp.blind_school_leak_in_essay
    {
        AgentVerdict::G1InterceptForceHold
    } else if inp.art11_doc_return_delay_days > 14 {
        AgentVerdict::G1ExpiredMandatoryReject
    } else if inp.is_adverse_or_disciplinary {
        AgentVerdict::G1InterceptForceHold
    } else {
        AgentVerdict::G3EligibleNormal
    };
    let fc = select_form_for_verdict(requested_form, verdict, inp.supplement_count, false);
    let disp = auto_pol.decide_disposition(
        verdict.nature(),
        Some(0.99),
        verdict == AgentVerdict::G3EligibleNormal && !inp.is_adverse_or_disciplinary,
        "DOM17-HR",
    );
    Ok((verdict, fc, disp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn dom17_interval_union_caps_concurrent_jobs_and_blocks_ai_autofinalize() {
        // Two jobs overlapping 182 days across a 2-year window -> 731 union days, NOT 913
        let union_days = compute_verified_career_days_union(&[(0, 731, 1.0), (100, 282, 1.0)]);
        assert_eq!(union_days, 731);

        let mut pol = AutomationPolicy::default();
        pol.enable_auto("OFF-HR-01", "AI기본법 제34조", Utc::now()).unwrap();
        let inflated = Dom17BlindHrRecruitInput {
            terminal_role: DeviceRole::PersonalPhone,
            profile: HiringAuditProfile::PublicBlindStrict,
            art4_3_prohibited_count: 0,
            blind_school_leak_in_essay: false,
            career_claimed_days: 913,
            career_intervals: vec![(0, 731, 1.0), (100, 282, 1.0)],
            is_adverse_or_disciplinary: true,
            art11_doc_return_delay_days: 7,
            epoch_privacy_locked: false,
            audio_prohibited_family_question: false,
            supplement_count: 0,
        };
        let (v, fc, disp) = evaluate_dom17_blind_hr(&inflated, OutputForm::ApprovalNotice, &pol).unwrap();
        assert_eq!(v, AgentVerdict::G2RejectContradictionLock);
        assert!(fc.overridden && fc.form == OutputForm::RejectNotice);
        assert!(matches!(disp, Disposition::PendingOfficer { .. }));

        // Private mode allows degree info in essay without false hold
        let private_ok = Dom17BlindHrRecruitInput {
            profile: HiringAuditProfile::Standard,
            blind_school_leak_in_essay: true,
            career_claimed_days: 731,
            is_adverse_or_disciplinary: false,
            ..inflated.clone()
        };
        assert_eq!(
            evaluate_dom17_blind_hr(&private_ok, OutputForm::ApprovalNotice, &pol).unwrap().0,
            AgentVerdict::G3EligibleNormal
        );
        assert_eq!(
            evaluate_dom17_blind_hr(&Dom17BlindHrRecruitInput { terminal_role: DeviceRole::CounterKiosk, ..inflated }, OutputForm::ApprovalNotice, &pol),
            Err(VaultError::KioskForbidden)
        );
    }
}
