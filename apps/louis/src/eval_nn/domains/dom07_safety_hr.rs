//! `DOM-07 (safety_hr)`: PTW/LOTO, Confined Space Art. 619-2 re-entry gas check, >= 10 m/s wind PolarityFlip, 52h vs 3-mo flex.

use lv_agent::gates::{select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};

pub const DOM07_FORMS: [&str; 6] = [
    "DOM07_F01_PTW_PERMIT",
    "DOM07_F02_GAS_CONFINED",
    "DOM07_F03_LOTO_AOP",
    "DOM07_F04_A0_TOPOLOGY",
    "DOM07_F05_RIGGING_HIGH",
    "DOM07_F06_WORK_HOURS_AUDIO",
];

#[derive(Debug, Clone)]
pub struct Dom07SafetyHrInput {
    pub o2_pct: f32,
    pub co2_pct: f32,
    pub co_ppm: f32,
    pub h2s_ppm: f32,
    pub reentered_after_break_without_retest: bool,
    pub loto_steps_in_order: bool,
    pub wind_gust_mps: f32,
    pub stop_order_active: bool,
    pub work_hours_week: f32,
    pub flex_3m_written_agreed: bool,
    pub flex_avg_hours_week: f32,
    pub continuous_rest_hours: f32,
    pub pregnant_or_minor_in_flex: bool,
    pub special_ext_approved: bool,
    pub supplement_count: u32,
}

pub fn evaluate_dom07_safety_hr(
    inp: &Dom07SafetyHrInput,
    requested_form: OutputForm,
) -> (AgentVerdict, FormChoice) {
    if !inp.loto_steps_in_order {
        let v = AgentVerdict::G3InterceptInterlockOrder;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    let gas_ok = (18.0..23.5).contains(&inp.o2_pct)
        && inp.co2_pct < 1.5
        && inp.co_ppm < 30.0
        && inp.h2s_ppm < 10.0
        && !inp.reentered_after_break_without_retest;
    let rigging_halt = inp.wind_gust_mps >= 10.0 || inp.stop_order_active;
    let hours_ok = if inp.work_hours_week <= 52.0 || inp.special_ext_approved {
        true
    } else {
        inp.flex_3m_written_agreed
            && !inp.pregnant_or_minor_in_flex
            && inp.work_hours_week <= 64.0
            && inp.flex_avg_hours_week <= 40.0
            && inp.continuous_rest_hours >= 11.0
    };
    if !gas_ok || rigging_halt || !hours_ok {
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
    fn dom07_confined_retest_wind_halt_and_flex_hours_boundaries() {
        let flex_ok = Dom07SafetyHrInput {
            o2_pct: 20.9,
            co2_pct: 0.04,
            co_ppm: 0.0,
            h2s_ppm: 0.0,
            reentered_after_break_without_retest: false,
            loto_steps_in_order: true,
            wind_gust_mps: 7.5,
            stop_order_active: false,
            work_hours_week: 58.0,
            flex_3m_written_agreed: true,
            flex_avg_hours_week: 40.0,
            continuous_rest_hours: 11.0,
            pregnant_or_minor_in_flex: false,
            special_ext_approved: false,
            supplement_count: 0,
        };
        assert_eq!(evaluate_dom07_safety_hr(&flex_ok, OutputForm::ApprovalNotice).0, AgentVerdict::G3EligibleNormal);
        assert_eq!(
            evaluate_dom07_safety_hr(&Dom07SafetyHrInput { continuous_rest_hours: 10.5, ..flex_ok.clone() }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G3ViolationHighRisk
        );
        assert_eq!(
            evaluate_dom07_safety_hr(&Dom07SafetyHrInput { reentered_after_break_without_retest: true, ..flex_ok.clone() }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G3ViolationHighRisk
        );
        let (v_wind, fc_wind) = evaluate_dom07_safety_hr(&Dom07SafetyHrInput { wind_gust_mps: 10.0, ..flex_ok }, OutputForm::ApprovalNotice);
        assert_eq!(v_wind, AgentVerdict::G3ViolationHighRisk);
        assert!(fc_wind.overridden && fc_wind.form == OutputForm::RejectNotice);
    }
}
