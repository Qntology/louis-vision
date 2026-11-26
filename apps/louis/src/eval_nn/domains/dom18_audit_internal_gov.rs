//! `DOM-18 (audit_board_gov)`: MOIS Ordinance No. 395 (eff. 2026-11-06) annual single-quote caps,
//! <= 5M KRW count exemption + sum inclusion, ZZD1 board redline cap, and rubber-stamp block.

use lv_agent::gates::{officer_approve, select_form_for_verdict, AgentVerdict, FormChoice, Justification, OfficerOutcome, OutputForm};

pub const DOM18_FORMS: [&str; 6] = [
    "DOM18_F01_SINGLE_QUOTE",
    "DOM18_F02_ANNUAL_CAP_LOG",
    "DOM18_F03_BOARD_MINUTES",
    "DOM18_F04_CORP_CERT_PACK",
    "DOM18_F05_WHISTLE_REPORT",
    "DOM18_F06_AUDIT_DISPOS",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteMethod2026 {
    SingleQuoteSmallSum,
    TwoPlusElectronicG2b,
    EmergencyDisasterSingle,
}

#[derive(Debug, Clone)]
pub struct PriorVendorContractRow {
    pub signed_ymd: u32,
    pub method: QuoteMethod2026,
    pub est_price_excl_vat_krw: i64,
}

/// Computes MOIS Ordinance 395 YTD totals: non-retroactive (`>= 20261106`), `<= 5M` excluded from count yet
/// summed into total amount, 2+ G2B excluded.
pub fn compute_rule395_ytd_totals(
    prior: &[PriorVendorContractRow],
    curr_ymd: u32,
    curr_method: QuoteMethod2026,
    curr_est_krw: i64,
) -> (u32, i64) {
    let mut count = 0u32;
    let mut sum_krw = 0i64;
    for (ymd, m, est) in prior
        .iter()
        .map(|r| (r.signed_ymd, r.method, r.est_price_excl_vat_krw))
        .chain(std::iter::once((curr_ymd, curr_method, curr_est_krw)))
    {
        if ymd >= 2026_1106 && m == QuoteMethod2026::SingleQuoteSmallSum {
            if est > 5_000_000 {
                count += 1;
            }
            sum_krw = sum_krw.saturating_add(est);
        }
    }
    (count, sum_krw)
}

#[derive(Debug, Clone)]
pub struct Dom18AuditInternalGovInput {
    pub is_metro_gov: bool,
    pub is_special_corp_50m: bool,
    pub corp_cert_valid: bool,
    pub conflict_art12_or_wage_arrears: bool,
    pub sme_bid_but_mid_corp: bool,
    pub prior_contracts: Vec<PriorVendorContractRow>,
    pub curr_ymd: u32,
    pub curr_method: QuoteMethod2026,
    pub curr_est_excl_vat_krw: i64,
    pub board_zzd1_revised_cap_krw: Option<i64>,
    pub cumulative_executed_krw: i64,
    pub whistleblower_identity_exposed: bool,
    pub supplement_count: u32,
}

pub fn evaluate_dom18_audit_internal_gov(
    inp: &Dom18AuditInternalGovInput,
    requested_form: OutputForm,
) -> (AgentVerdict, FormChoice) {
    if inp.whistleblower_identity_exposed {
        let v = AgentVerdict::G4BlockInternalLeak;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.conflict_art12_or_wage_arrears || inp.sme_bid_but_mid_corp {
        let v = AgentVerdict::G3ViolationHighRisk;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if let Some(cap) = inp.board_zzd1_revised_cap_krw {
        if inp.cumulative_executed_krw > cap {
            let v = AgentVerdict::G3ViolationHighRisk;
            return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
        }
    }
    if !inp.corp_cert_valid {
        let v = AgentVerdict::G1ExpiredMandatoryReject;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    let single_limit = if inp.is_special_corp_50m { 50_000_000 } else { 20_000_000 };
    let (ytd_cnt, ytd_sum) = compute_rule395_ytd_totals(
        &inp.prior_contracts,
        inp.curr_ymd,
        inp.curr_method,
        inp.curr_est_excl_vat_krw,
    );
    let (cnt_cap, sum_cap) = if inp.is_metro_gov {
        (7u32, 300_000_000i64)
    } else {
        (5u32, 200_000_000i64)
    };
    if (inp.curr_method == QuoteMethod2026::SingleQuoteSmallSum
        && inp.curr_est_excl_vat_krw > single_limit)
        || ytd_cnt > cnt_cap
        || ytd_sum > sum_cap
    {
        let v = AgentVerdict::G3ViolationRelatedVendorSplit;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    let v = AgentVerdict::G3EligibleNormal;
    (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false))
}

pub fn verify_dom18_audit_override(
    verdict: AgentVerdict,
    justification: Option<&Justification>,
    lead_wallet_sig_verified: bool,
) -> Result<OfficerOutcome, AgentVerdict> {
    if matches!(justification, Some(Justification::ApplyLawException { .. })) && !lead_wallet_sig_verified {
        return Err(AgentVerdict::G3InterceptRubberStamp);
    }
    officer_approve(verdict, justification)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dom18_rule395_non_retroactive_5m_threshold_and_rubber_stamp() {
        let rows = vec![
            PriorVendorContractRow {
                signed_ymd: 2026_1105, // pre-effective -> excluded!
                method: QuoteMethod2026::SingleQuoteSmallSum,
                est_price_excl_vat_krw: 19_000_000,
            },
            PriorVendorContractRow {
                signed_ymd: 2026_1110,
                method: QuoteMethod2026::SingleQuoteSmallSum,
                est_price_excl_vat_krw: 5_000_000, // <= 5M: excluded from count, summed into amount!
            },
            PriorVendorContractRow {
                signed_ymd: 2026_1112,
                method: QuoteMethod2026::TwoPlusElectronicG2b, // 2+ G2B -> excluded!
                est_price_excl_vat_krw: 40_000_000,
            },
        ];
        let (cnt, sum) = compute_rule395_ytd_totals(&rows, 2026_1115, QuoteMethod2026::SingleQuoteSmallSum, 19_500_000);
        assert_eq!(cnt, 1);
        assert_eq!(sum, 24_500_000);

        let law_exc = Justification::ApplyLawException {
            team_lead_cosign: true,
            rule_ref: "행안부예규 제395호 긴급예외".into(),
        };
        assert_eq!(
            verify_dom18_audit_override(AgentVerdict::G3ViolationRelatedVendorSplit, Some(&law_exc), false),
            Err(AgentVerdict::G3InterceptRubberStamp)
        );
        assert_eq!(
            verify_dom18_audit_override(AgentVerdict::G3ViolationRelatedVendorSplit, Some(&law_exc), true),
            Ok(OfficerOutcome::Approved { special_audit_flag: true })
        );
    }
}
