//! `DOM-03 (insolvency_legal)`: Personal rehabilitation / bankruptcy creditor list, branch numbers (`4-1`), and liquidation value.

use lv_agent::gates::{select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};
use lv_router::vault::{DeviceRole, VaultError};

pub const DOM03_FORMS: [&str; 6] = [
    "DOM03_F01_CREDITOR_LIST",
    "DOM03_F02_DEBT_CERT_FAX",
    "DOM03_F03_ASSET_LIQUIDATION",
    "DOM03_F04_INCOME_REPAY_PLAN",
    "DOM03_F05_SERVICE_NOTICE",
    "DOM03_F06_TRUSTEE_AUDIO",
];

#[derive(Debug, Clone)]
pub struct Dom03InsolvencyInput {
    pub terminal_role: DeviceRole,
    pub double_counted_subrogation_branch: bool,
    pub total_repay_pv_krw: i64,
    pub liquidation_value_krw: i64,
    pub claimed_living_cost_krw: i64,
    pub median_60pct_living_cap_krw: i64,
    pub days_past_correction_deadline: i32,
    pub audio_hidden_crypto_or_preferential: bool,
    pub supplement_count: u32,
}

pub fn evaluate_dom03_insolvency_legal(
    inp: &Dom03InsolvencyInput,
    requested_form: OutputForm,
) -> Result<(AgentVerdict, FormChoice), VaultError> {
    if matches!(inp.terminal_role, DeviceRole::CounterKiosk) {
        return Err(VaultError::KioskForbidden);
    }
    if inp.audio_hidden_crypto_or_preferential {
        let v = AgentVerdict::G3ViolationHighRisk;
        return Ok((v, select_form_for_verdict(requested_form, v, inp.supplement_count, false)));
    }
    if inp.days_past_correction_deadline > 0 {
        let v = AgentVerdict::G1ExpiredMandatoryReject;
        return Ok((v, select_form_for_verdict(requested_form, v, inp.supplement_count, false)));
    }
    if inp.double_counted_subrogation_branch
        || inp.total_repay_pv_krw < inp.liquidation_value_krw
        || inp.claimed_living_cost_krw > inp.median_60pct_living_cap_krw
    {
        let v = AgentVerdict::G2RejectContradictionLock;
        return Ok((v, select_form_for_verdict(requested_form, v, inp.supplement_count, false)));
    }
    let v = AgentVerdict::G3EligibleNormal;
    Ok((v, select_form_for_verdict(requested_form, v, inp.supplement_count, false)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dom03_kiosk_block_branch_double_count_and_d0_deadline() {
        let ok = Dom03InsolvencyInput {
            terminal_role: DeviceRole::PersonalPhone,
            double_counted_subrogation_branch: false,
            total_repay_pv_krw: 42_000_000,
            liquidation_value_krw: 38_000_000,
            claimed_living_cost_krw: 1_450_000,
            median_60pct_living_cap_krw: 1_530_000,
            days_past_correction_deadline: 0,
            audio_hidden_crypto_or_preferential: false,
            supplement_count: 0,
        };
        assert_eq!(evaluate_dom03_insolvency_legal(&ok, OutputForm::ApprovalNotice).unwrap().0, AgentVerdict::G3EligibleNormal);
        assert_eq!(
            evaluate_dom03_insolvency_legal(&Dom03InsolvencyInput { terminal_role: DeviceRole::CounterKiosk, ..ok.clone() }, OutputForm::ApprovalNotice),
            Err(VaultError::KioskForbidden)
        );
        assert_eq!(
            evaluate_dom03_insolvency_legal(&Dom03InsolvencyInput { days_past_correction_deadline: 1, ..ok }, OutputForm::ApprovalNotice).unwrap().0,
            AgentVerdict::G1ExpiredMandatoryReject
        );
    }
}
