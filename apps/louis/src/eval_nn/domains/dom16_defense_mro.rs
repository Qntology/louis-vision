//! `DOM-16 (defense_ram_mro)`: Depot MRO ECP Class I/II, AS6174 anti-counterfeit, NSN-13 repair,
//! 5.5 px/mm 3D plaque readability, 45 deg tactical monitor zeroize, and 3-step runup interlock.

use lv_agent::gates::{select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};

pub const DOM16_FORMS: [&str; 6] = [
    "DOM16_F01_MRO_WORK_ORDER",
    "DOM16_F02_ECP_WAIVER",
    "DOM16_F03_COC_8130_TAG",
    "DOM16_F04_3D_PLAQUE_FUSE",
    "DOM16_F05_RUNUP_INTERLOCK",
    "DOM16_F06_TESTCELL_AUDIO",
];

pub fn repair_and_verify_nsn13(raw: &str) -> Option<String> {
    let fixed: String = raw
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '-' | '.' | '_'))
        .map(|c| match c.to_ascii_uppercase() {
            'O' | 'Q' => '0',
            'I' | 'L' => '1',
            'Z' => '2',
            'S' => '5',
            'G' => '6',
            'B' => '8',
            other => other,
        })
        .collect();
    if fixed.len() != 13 || !fixed.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}-{}-{}", &fixed[0..4], &fixed[4..6], &fixed[6..13]))
}

#[derive(Debug, Clone)]
pub struct Dom16DefenseMroInput {
    pub runup_interlock_in_order: bool,
    pub plaque_px_per_mm: f32,
    pub tactical_screen_nonzero_pixels: usize,
    pub torque_nm: f32, // spec 115..=125 N*m
    pub torque_wrench_calib_expired: bool,
    pub mechanic_equals_qa: bool,
    pub matches_baseline_pn: bool,
    pub approved_ecp_or_waiver_lot_valid: bool,
    pub critical_safety_defect_waiver_attempted: bool,
    pub unauthorized_broker_or_dup_serial: bool,
    pub llp_remaining_cycles: i32,
    pub nsn13_raw: String,
    pub egoexo_rmse_mm: f32, // <= 15.0 mm
    pub audio_bearing_chatter_or_conceal: bool,
    pub supplement_count: u32,
}

pub fn evaluate_dom16_defense_mro(
    inp: &Dom16DefenseMroInput,
    requested_form: OutputForm,
) -> (AgentVerdict, FormChoice) {
    if !inp.runup_interlock_in_order {
        let v = AgentVerdict::G3InterceptInterlockOrder;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.tactical_screen_nonzero_pixels > 0 {
        let v = AgentVerdict::G4BlockInternalLeak;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.critical_safety_defect_waiver_attempted
        || inp.mechanic_equals_qa
        || inp.unauthorized_broker_or_dup_serial
        || inp.llp_remaining_cycles <= 0
        || inp.audio_bearing_chatter_or_conceal
    {
        let v = AgentVerdict::G3ViolationHighRisk;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.torque_wrench_calib_expired {
        let v = AgentVerdict::G1ExpiredMandatoryReject;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.plaque_px_per_mm < 5.5 {
        let v = AgentVerdict::G2RequireRepairReceipt;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if !(115.0..=125.0).contains(&inp.torque_nm)
        || (!inp.matches_baseline_pn && !inp.approved_ecp_or_waiver_lot_valid)
        || repair_and_verify_nsn13(&inp.nsn13_raw).is_none()
        || inp.egoexo_rmse_mm > 15.0
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
    fn dom16_nsn13_plaque_dpi_zeroize_and_critical_defect_non_waivable() {
        assert_eq!(repair_and_verify_nsn13("284O-37-I04829I").as_deref(), Some("2840-37-1048291"));
        let ok = Dom16DefenseMroInput {
            runup_interlock_in_order: true,
            plaque_px_per_mm: 6.4,
            tactical_screen_nonzero_pixels: 0,
            torque_nm: 120.0,
            torque_wrench_calib_expired: false,
            mechanic_equals_qa: false,
            matches_baseline_pn: true,
            approved_ecp_or_waiver_lot_valid: true,
            critical_safety_defect_waiver_attempted: false,
            unauthorized_broker_or_dup_serial: false,
            llp_remaining_cycles: 450,
            nsn13_raw: "284O-37-I04829I".into(),
            egoexo_rmse_mm: 9.2,
            audio_bearing_chatter_or_conceal: false,
            supplement_count: 0,
        };
        assert_eq!(evaluate_dom16_defense_mro(&ok, OutputForm::ApprovalNotice).0, AgentVerdict::G3EligibleNormal);
        assert_eq!(
            evaluate_dom16_defense_mro(&Dom16DefenseMroInput { plaque_px_per_mm: 4.9, ..ok.clone() }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G2RequireRepairReceipt
        );
        assert_eq!(
            evaluate_dom16_defense_mro(&Dom16DefenseMroInput { critical_safety_defect_waiver_attempted: true, ..ok }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G3ViolationHighRisk
        );
    }
}
