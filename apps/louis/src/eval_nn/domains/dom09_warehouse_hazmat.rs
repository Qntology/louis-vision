//! `DOM-09 (warehouse_hazmat_pod)`: Customs Art. 248/252 bonded release, IMDG 42-24, ISO 6346 `rem==10 -> '0'`, fragile slowdown.

use lv_agent::gates::{select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};

pub const DOM09_FORMS: [&str; 6] = [
    "DOM09_F01_BONDED_REL",
    "DOM09_F02_DO_POD",
    "DOM09_F03_DGD_2026",
    "DOM09_F04_GHS_MSDS",
    "DOM09_F05_3D_CARGO_LBL",
    "DOM09_F06_HAZMAT_LOTO",
];

pub fn iso6346_char_value(c: char) -> Option<u32> {
    if c.is_ascii_digit() {
        return c.to_digit(10);
    }
    let idx = (c.to_ascii_uppercase() as u32).checked_sub('A' as u32)?;
    if idx >= 26 {
        return None;
    }
    let raw = 10 + idx;
    Some(raw + (raw >= 11) as u32 + (raw >= 21) as u32 + (raw >= 31) as u32)
}

/// `MSKU431829` -> `'5'`, `MSCU113466` -> `rem == 10 -> '0'` (never `"10"`).
pub fn compute_iso6346_check_digit(prefix10: &str) -> Option<char> {
    let chars: Vec<char> = prefix10.chars().collect();
    if chars.len() != 10
        || !chars[0..3].iter().all(|c| c.is_ascii_alphabetic())
        || !matches!(chars[3].to_ascii_uppercase(), 'U' | 'J' | 'Z' | 'R')
        || !chars[4..10].iter().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let mut sum = 0u32;
    for (i, &ch) in chars.iter().enumerate() {
        sum += iso6346_char_value(ch)? * (1u32 << i);
    }
    let rem = sum % 11;
    char::from_digit(if rem == 10 { 0 } else { rem }, 10)
}

#[derive(Debug, Clone)]
pub struct Dom09WarehouseHazmatInput {
    pub container_code_11: String,
    pub clearance_accepted_art248: bool,
    pub duty_collateral_provided_art252: bool,
    pub pre_release_approval_no: Option<String>,
    pub un_code: Option<String>,
    pub dgd_version_42_24: bool,
    pub dgd_msds_matched: bool,
    pub sp978_three_dates_temp_complete: bool,
    pub cargo_label_px_per_mm: f32,
    pub fragile_symbol_present: bool,
    pub grip_force_scale: f32,
    pub accel_cap_scale: f32,
    pub vent_ach: f32,
    pub static_ground_ohm: f32,
    pub loto_in_order: bool,
    pub supplement_count: u32,
}

pub fn evaluate_dom09_warehouse_hazmat(
    inp: &Dom09WarehouseHazmatInput,
    requested_form: OutputForm,
) -> (AgentVerdict, FormChoice) {
    if !inp.loto_in_order || inp.vent_ach < 12.0 || inp.static_ground_ohm > 10.0 {
        let v = AgentVerdict::G3InterceptInterlockOrder;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.cargo_label_px_per_mm < 4.0 {
        let v = AgentVerdict::G2RequireRepairReceipt;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    let art252_ok = inp.duty_collateral_provided_art252
        && inp.pre_release_approval_no.as_deref().is_some_and(|s| !s.trim().is_empty());
    if !inp.clearance_accepted_art248 && !art252_ok {
        let v = AgentVerdict::G3ViolationHighRisk;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    let valid_iso = inp.container_code_11.len() == 11
        && compute_iso6346_check_digit(&inp.container_code_11[..10])
            == inp.container_code_11.chars().nth(10);
    let legacy_un3171 = inp.un_code.as_deref().is_some_and(|u| u.eq_ignore_ascii_case("UN 3171") || u.eq_ignore_ascii_case("UN3171"));
    let sp978_needed = inp.un_code.as_deref().is_some_and(|u| u.contains("1361") || u.contains("1362"));
    let fragile_ok = !inp.fragile_symbol_present
        || ((inp.grip_force_scale - 0.65).abs() <= 1e-4 && (inp.accel_cap_scale - 0.55).abs() <= 1e-4);

    if !valid_iso
        || legacy_un3171
        || !inp.dgd_version_42_24
        || !inp.dgd_msds_matched
        || (sp978_needed && !inp.sp978_three_dates_temp_complete)
        || !fragile_ok
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
    fn dom09_iso6346_rem10_art252_and_imdg_42_24() {
        assert_eq!(compute_iso6346_check_digit("MSKU431829"), Some('5'));
        assert_eq!(compute_iso6346_check_digit("MSCU113466"), Some('0'));

        let ok = Dom09WarehouseHazmatInput {
            container_code_11: "MSCU1134660".into(),
            clearance_accepted_art248: false,
            duty_collateral_provided_art252: true,
            pre_release_approval_no: Some("ICN-252-001".into()),
            un_code: Some("UN 3551".into()),
            dgd_version_42_24: true,
            dgd_msds_matched: true,
            sp978_three_dates_temp_complete: true,
            cargo_label_px_per_mm: 5.2,
            fragile_symbol_present: true,
            grip_force_scale: 0.65,
            accel_cap_scale: 0.55,
            vent_ach: 14.0,
            static_ground_ohm: 4.5,
            loto_in_order: true,
            supplement_count: 0,
        };
        assert_eq!(evaluate_dom09_warehouse_hazmat(&ok, OutputForm::ApprovalNotice).0, AgentVerdict::G3EligibleNormal);
        assert_eq!(
            evaluate_dom09_warehouse_hazmat(&Dom09WarehouseHazmatInput { un_code: Some("UN 3171".into()), ..ok.clone() }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G2RejectContradictionLock
        );
        assert_eq!(
            evaluate_dom09_warehouse_hazmat(&Dom09WarehouseHazmatInput { cargo_label_px_per_mm: 3.8, ..ok }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G2RequireRepairReceipt
        );
    }
}
