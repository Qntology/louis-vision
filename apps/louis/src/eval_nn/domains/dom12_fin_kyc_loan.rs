//! `DOM-12 (fin_kyc_loan)`: 3-Stage 25% indirect UBO, BizRegNoKr10 OCR repair, 2026 Accountability Map, Kiosk hard lock.

use lv_agent::gates::{select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};
use lv_router::vault::{DeviceRole, VaultError};

pub const DOM12_FORMS: [&str; 6] = [
    "DOM12_F01_LOAN_APPROVAL",
    "DOM12_F02_SHARE_LEDGER",
    "DOM12_F03_CORP_REG_BIZ",
    "DOM12_F04_CDD_EDD_FORM",
    "DOM12_F05_RESP_MAP_CHK",
    "DOM12_F06_CREDIT_COMM",
];

pub fn verify_biz_reg_no_kr10(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '-' | '.' | '_' | '/'))
        .map(|c| match c.to_ascii_uppercase() {
            'O' | 'Q' => '0',
            'I' | 'L' => '1',
            'Z' => '2',
            'S' => '5',
            'B' => '8',
            other => other,
        })
        .collect();
    if cleaned.len() != 10 || !cleaned.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let d: Vec<u32> = cleaned.chars().filter_map(|c| c.to_digit(10)).collect();
    const WEIGHTS: [u32; 9] = [1, 3, 7, 1, 3, 7, 1, 3, 5];
    let sum: u32 = (0..9).map(|i| d[i] * WEIGHTS[i]).sum::<u32>() + (d[8] * 5) / 10;
    (((10 - (sum % 10)) % 10) == d[9]).then_some(cleaned)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UboStageClaimed {
    ExemptPublicOrListed,
    Stage1NaturalOver25Pct, // e.g. 10% direct + 50%*30% indirect = 25.00% >= 2500 bps
    Stage2ControllingOrMax,
    Stage3CeoFallback,
}

#[derive(Debug, Clone)]
pub struct Dom12FinKycLoanInput {
    pub terminal_role: DeviceRole,
    pub raw_rrn_or_keypad_leak: bool,
    pub covert_nominee_or_homoglyph_count: u32,
    pub registry_issued_shares: u64,
    pub shareholder_ledger_sum_shares: u64,
    pub claimed_ubo_stage: UboStageClaimed,
    pub max_natural_effective_share_bps: u32,
    pub has_identifiable_stage2_natural_max: bool,
    pub stopped_at_corporate_shareholder: bool,
    pub high_risk_edd_triggered: bool,
    pub edd_source_of_funds_attached: bool,
    pub accountability_code: Option<String>,
    pub accountability_map_current: bool,
    pub exec_wallet_matches_assigned_officer: bool,
    pub control_action_logged: bool,
    pub audio_nominee_or_side_contract_detected: bool,
    pub supplement_count: u32,
}

pub fn evaluate_dom12_fin_kyc_loan(
    inp: &Dom12FinKycLoanInput,
    requested_form: OutputForm,
) -> Result<(AgentVerdict, FormChoice), VaultError> {
    if matches!(inp.terminal_role, DeviceRole::CounterKiosk) {
        return Err(VaultError::KioskForbidden);
    }
    if inp.raw_rrn_or_keypad_leak {
        let v = AgentVerdict::G4BlockInternalLeak;
        return Ok((v, select_form_for_verdict(requested_form, v, inp.supplement_count, false)));
    }
    if inp.covert_nominee_or_homoglyph_count > 0 || inp.audio_nominee_or_side_contract_detected {
        let v = AgentVerdict::G3ViolationHighRisk;
        return Ok((v, select_form_for_verdict(requested_form, v, inp.supplement_count, false)));
    }
    if !inp.accountability_code.as_deref().is_some_and(|c| !c.trim().is_empty())
        || !inp.control_action_logged
    {
        let v = AgentVerdict::G1InterceptForceHold;
        return Ok((v, select_form_for_verdict(requested_form, v, inp.supplement_count, false)));
    }
    if !inp.accountability_map_current
        || !inp.exec_wallet_matches_assigned_officer
        || inp.registry_issued_shares != inp.shareholder_ledger_sum_shares
    {
        let v = AgentVerdict::G2RejectContradictionLock;
        return Ok((v, select_form_for_verdict(requested_form, v, inp.supplement_count, false)));
    }
    if inp.claimed_ubo_stage != UboStageClaimed::ExemptPublicOrListed {
        let stage_ok = !inp.stopped_at_corporate_shareholder
            && match inp.claimed_ubo_stage {
                UboStageClaimed::ExemptPublicOrListed => true,
                UboStageClaimed::Stage1NaturalOver25Pct => inp.max_natural_effective_share_bps >= 2_500,
                UboStageClaimed::Stage2ControllingOrMax => {
                    inp.max_natural_effective_share_bps < 2_500 && inp.has_identifiable_stage2_natural_max
                }
                UboStageClaimed::Stage3CeoFallback => {
                    inp.max_natural_effective_share_bps < 2_500 && !inp.has_identifiable_stage2_natural_max
                }
            };
        if !stage_ok {
            let v = AgentVerdict::G3ViolationHighRisk;
            return Ok((v, select_form_for_verdict(requested_form, v, inp.supplement_count, false)));
        }
    }
    if inp.high_risk_edd_triggered && !inp.edd_source_of_funds_attached {
        let v = if inp.supplement_count == 0 {
            AgentVerdict::G3HoldSimpleReceiptMissing
        } else {
            AgentVerdict::G3ViolationHighRisk
        };
        return Ok((v, select_form_for_verdict(requested_form, v, inp.supplement_count, true)));
    }
    let v = AgentVerdict::G3EligibleNormal;
    Ok((v, select_form_for_verdict(requested_form, v, inp.supplement_count, false)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dom12_ubo_25pct_stage_skip_accountability_and_kiosk_block() {
        assert_eq!(verify_biz_reg_no_kr10("11O-8I-41272").as_deref(), Some("1108141272"));
        let ok = Dom12FinKycLoanInput {
            terminal_role: DeviceRole::PersonalPhone,
            raw_rrn_or_keypad_leak: false,
            covert_nominee_or_homoglyph_count: 0,
            registry_issued_shares: 100_000,
            shareholder_ledger_sum_shares: 100_000,
            claimed_ubo_stage: UboStageClaimed::Stage1NaturalOver25Pct,
            max_natural_effective_share_bps: 2_500, // 10% + 50%*30% = 25.00%
            has_identifiable_stage2_natural_max: true,
            stopped_at_corporate_shareholder: false,
            high_risk_edd_triggered: false,
            edd_source_of_funds_attached: false,
            accountability_code: Some("CE-LOAN-2026-04".into()),
            accountability_map_current: true,
            exec_wallet_matches_assigned_officer: true,
            control_action_logged: true,
            audio_nominee_or_side_contract_detected: false,
            supplement_count: 0,
        };
        assert_eq!(evaluate_dom12_fin_kyc_loan(&ok, OutputForm::ApprovalNotice).unwrap().0, AgentVerdict::G3EligibleNormal);
        assert_eq!(
            evaluate_dom12_fin_kyc_loan(&Dom12FinKycLoanInput {
                claimed_ubo_stage: UboStageClaimed::Stage3CeoFallback,
                max_natural_effective_share_bps: 2_000,
                has_identifiable_stage2_natural_max: true,
                ..ok.clone()
            }, OutputForm::ApprovalNotice).unwrap().0,
            AgentVerdict::G3ViolationHighRisk
        );
        assert_eq!(
            evaluate_dom12_fin_kyc_loan(&Dom12FinKycLoanInput { terminal_role: DeviceRole::CounterKiosk, ..ok }, OutputForm::ApprovalNotice),
            Err(VaultError::KioskForbidden)
        );
    }
}
