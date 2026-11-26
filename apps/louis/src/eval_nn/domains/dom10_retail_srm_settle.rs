//! `DOM-10 (retail_srm_settle)`: 2026 Retail Distribution Act (20d/35d deadlines), 50% promo cap,
//! `1-Hop FrictionTrap` choice veto, and `CROSS_ANCHOR_BAND = 0.03`.

use lv_agent::gates::{select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};
use lv_domain::retail_srm::RetailContractKind;

pub const DOM10_FORMS: [&str; 6] = [
    "DOM10_F01_SRM_CONTRACT",
    "DOM10_F02_ORDER_RECV",
    "DOM10_F03_PROMO_COUPON",
    "DOM10_F04_MONTHLY_XLS",
    "DOM10_F05_RETURN_VOC",
    "DOM10_F06_PORTAL_FUNNEL",
];

pub const CROSS_ANCHOR_BAND: f32 = 0.03;

#[derive(Debug, Clone)]
pub struct ChoiceFunnelTelemetry {
    pub z_choice: f32,
    pub latency_ms: u64,
    pub rage_click_or_backtrack: bool,
    pub abandon_rate_pct: f32,
    pub is_fast_track_complete: bool,
    pub is_compare_tab_close: bool,
}

pub fn is_one_hop_friction_trap(ev: &ChoiceFunnelTelemetry) -> bool {
    if ev.is_fast_track_complete || ev.is_compare_tab_close {
        return false;
    }
    ev.z_choice > 2.0
        && ev.latency_ms <= 8_000
        && (ev.rage_click_or_backtrack || ev.abandon_rate_pct >= 50.0)
}

#[derive(Debug, Clone)]
pub struct Dom10RetailSrmInput {
    pub contract_kind: RetailContractKind,
    pub elapsed_days: i64,
    pub ftc_deadline_exception_approved: bool,
    pub promo_total_krw: i64,
    pub vendor_promo_burden_krw: i64,
    pub promo_written_pre_agreed: bool,
    pub unilateral_return_without_proof: bool,
    pub funnel_event: Option<ChoiceFunnelTelemetry>,
    pub supplement_count: u32,
}

pub fn evaluate_dom10_retail_srm(
    inp: &Dom10RetailSrmInput,
    requested_form: OutputForm,
) -> (AgentVerdict, FormChoice) {
    if !inp.promo_written_pre_agreed || inp.unilateral_return_without_proof {
        let v = AgentVerdict::G2RejectContradictionLock;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.funnel_event.as_ref().is_some_and(is_one_hop_friction_trap) {
        let v = AgentVerdict::G3ViolationHighRisk;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.promo_total_krw > 0 && inp.vendor_promo_burden_krw * 2 > inp.promo_total_krw {
        let v = if inp.supplement_count == 0 {
            AgentVerdict::G3HoldSimpleReceiptMissing
        } else {
            AgentVerdict::G3ViolationHighRisk
        };
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, true));
    }
    let limit = match inp.contract_kind {
        RetailContractKind::DirectPurchaseItemwise => 35,
        RetailContractKind::DirectPurchaseMonthly
        | RetailContractKind::SpecialPurchase
        | RetailContractKind::Consignment => 20,
    };
    if !inp.ftc_deadline_exception_approved && (inp.elapsed_days < 0 || inp.elapsed_days > limit) {
        let v = AgentVerdict::G1ExpiredMandatoryReject;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    let v = AgentVerdict::G3EligibleNormal;
    (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dom10_20d_35d_promo_cap_and_friction_trap_vs_fast_track() {
        let ok = Dom10RetailSrmInput {
            contract_kind: RetailContractKind::DirectPurchaseItemwise,
            elapsed_days: 35,
            ftc_deadline_exception_approved: false,
            promo_total_krw: 20_000_000,
            vendor_promo_burden_krw: 10_000_000,
            promo_written_pre_agreed: true,
            unilateral_return_without_proof: false,
            funnel_event: Some(ChoiceFunnelTelemetry {
                z_choice: 2.5,
                latency_ms: 3_000,
                rage_click_or_backtrack: false,
                abandon_rate_pct: 5.0,
                is_fast_track_complete: true,
                is_compare_tab_close: false,
            }),
            supplement_count: 0,
        };
        assert_eq!(evaluate_dom10_retail_srm(&ok, OutputForm::ApprovalNotice).0, AgentVerdict::G3EligibleNormal);
        assert_eq!(
            evaluate_dom10_retail_srm(&Dom10RetailSrmInput { elapsed_days: 36, ..ok.clone() }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G1ExpiredMandatoryReject
        );
        assert_eq!(
            evaluate_dom10_retail_srm(&Dom10RetailSrmInput {
                funnel_event: Some(ChoiceFunnelTelemetry {
                    z_choice: 2.8,
                    latency_ms: 4_500,
                    rage_click_or_backtrack: true,
                    abandon_rate_pct: 62.0,
                    is_fast_track_complete: false,
                    is_compare_tab_close: false,
                }),
                ..ok
            }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G3ViolationHighRisk
        );
    }
}
