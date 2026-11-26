//! `DOM-15 (eng_ecn_bom)`: PSM MOC Class 1-3 vs Replacement-in-Kind exemption, Temporary MOC expiry,
//! `A0` ZZD1 redline (`3.5 <= 4.2 bar`), active BOM (`<w:strike>`/`OBSOLETE` excluded) == ZZS1, PSSR, and twin abstention.

use lv_agent::gates::{select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};

pub const DOM15_FORMS: [&str; 6] = [
    "DOM15_F01_ECR_MOC_REQ",
    "DOM15_F02_ECN_RELEASE",
    "DOM15_F03_A0_REDLINE_PID",
    "DOM15_F04_MULTILEVEL_BOM",
    "DOM15_F05_LEGEND_PSSR",
    "DOM15_F06_HAZOP_AUDIO",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MocClassKind {
    ReplacementInKind,
    TemporaryMoc { days_past_restore_deadline: i32 },
    PermanentClass1Or2,
}

#[derive(Debug, Clone)]
pub struct Dom15EngEcnBomInput {
    pub moc_kind: MocClassKind,
    pub moc_committee_approved: bool,
    pub pssr_completed: bool,
    pub drawing_is_master_rev: bool,
    pub signer_sector_authorized: bool,
    pub title_block_5_of_5: bool,
    pub zzd1_redline_pressure_bar: f32,
    pub design_limit_bar: f32,
    pub zzs1_hoist_count: u32,
    pub active_bom_lines: Vec<(String, u32, bool, bool)>, // (sku, qty, is_struck, is_obsolete)
    pub twin_identify_gap: f32,
    pub audio_unauthorized_load_order: bool,
    pub supplement_count: u32,
}

pub fn active_bom_quantity_sum(lines: &[(String, u32, bool, bool)]) -> u32 {
    lines
        .iter()
        .filter(|(_, _, struck, obsolete)| !*struck && !*obsolete)
        .map(|(_, q, _, _)| *q)
        .sum()
}

pub fn evaluate_dom15_eng_ecn_bom(
    inp: &Dom15EngEcnBomInput,
    requested_form: OutputForm,
) -> (AgentVerdict, FormChoice) {
    if !inp.signer_sector_authorized || inp.audio_unauthorized_load_order {
        let v = AgentVerdict::G3ViolationHighRisk;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if let MocClassKind::TemporaryMoc { days_past_restore_deadline } = inp.moc_kind {
        if days_past_restore_deadline > 0 {
            let v = AgentVerdict::G1ExpiredMandatoryReject;
            return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
        }
    }
    if !inp.title_block_5_of_5 || inp.twin_identify_gap < 1.0 {
        let v = AgentVerdict::G1InterceptForceHold;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    let needs_moc_pssr = matches!(
        inp.moc_kind,
        MocClassKind::PermanentClass1Or2 | MocClassKind::TemporaryMoc { .. }
    );
    if needs_moc_pssr && (!inp.moc_committee_approved || !inp.pssr_completed) {
        let v = AgentVerdict::G1InterceptForceHold;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    let bom_qty = active_bom_quantity_sum(&inp.active_bom_lines);
    if !inp.drawing_is_master_rev
        || inp.zzd1_redline_pressure_bar > inp.design_limit_bar
        || bom_qty != inp.zzs1_hoist_count
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
    fn dom15_rik_exempt_bom_strike_exclusion_and_twin_abstain() {
        let rik = Dom15EngEcnBomInput {
            moc_kind: MocClassKind::ReplacementInKind,
            moc_committee_approved: false, // Exempt for RIK -> 0 false rejections
            pssr_completed: false,
            drawing_is_master_rev: true,
            signer_sector_authorized: true,
            title_block_5_of_5: true,
            zzd1_redline_pressure_bar: 3.5,
            design_limit_bar: 4.2,
            zzs1_hoist_count: 14,
            active_bom_lines: vec![
                ("HOIST-500".into(), 14, false, false),
                ("HOIST-OLD".into(), 4, true, false),  // struck -> excluded
                ("HOIST-OBS".into(), 2, false, true),  // obsolete -> excluded
            ],
            twin_identify_gap: 1.8,
            audio_unauthorized_load_order: false,
            supplement_count: 0,
        };
        assert_eq!(evaluate_dom15_eng_ecn_bom(&rik, OutputForm::ApprovalNotice).0, AgentVerdict::G3EligibleNormal);
        assert_eq!(
            evaluate_dom15_eng_ecn_bom(&Dom15EngEcnBomInput { twin_identify_gap: 0.4, ..rik.clone() }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G1InterceptForceHold
        );
        assert_eq!(
            evaluate_dom15_eng_ecn_bom(&Dom15EngEcnBomInput {
                moc_kind: MocClassKind::TemporaryMoc { days_past_restore_deadline: 1 },
                ..rik
            }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G1ExpiredMandatoryReject
        );
    }
}
