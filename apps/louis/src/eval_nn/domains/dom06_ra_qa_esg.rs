//! `DOM-06 (ra_qa_esg)`: Manufacturing CoA / Mill Sheet (`10^3` vs `10^4`) & 2026 EU CBAM `50 tonnes` De Minimis.

use lv_agent::gates::{select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};
use lv_doc::writer::guard::N2sfGrade;

pub const DOM06_FORMS: [&str; 6] = [
    "DOM06_F01_COA_MILL_SHEET",
    "DOM06_F02_CBAM_COMM_SHEET",
    "DOM06_F03_CALIB_SIGN_CERT",
    "DOM06_F04_PROC_FLOW_DIAG",
    "DOM06_F05_COIL_DRUM_3D",
    "DOM06_F06_NDT_AUDIT_AUDIO",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CbamSectorCategory {
    SteelAluminumCementFertilizer, // Subject to 50,000 kg (50 tonnes) annual De Minimis exemption
    ElectricityOrHydrogen,         // Excluded from De Minimis (mandatory declaration even < 50 t)
}

#[derive(Debug, Clone)]
pub struct Dom06RaQaEsgInput {
    pub sector: CbamSectorCategory,
    pub annual_import_net_mass_kg: u64,
    pub verifier_report_attached: bool,
    pub coa_microbial_cfu_text: String, // "1.2x10^3" passes, "1.2x10^4" or unformatted fail
    pub calib_sig_valid: bool,
    pub catalyst_grade: N2sfGrade,
    pub target_zone: N2sfGrade,
    pub audio_ndt_defect_ping: bool,
    pub supplement_count: u32,
}

pub fn evaluate_dom06_ra_qa_esg(
    inp: &Dom06RaQaEsgInput,
    requested_form: OutputForm,
) -> (AgentVerdict, FormChoice) {
    if inp.catalyst_grade == N2sfGrade::Classified && inp.target_zone == N2sfGrade::Open {
        let v = AgentVerdict::G3ViolationHighRisk;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if !inp.calib_sig_valid {
        let v = AgentVerdict::G1SignInvalidRetry;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    let coa_ok = inp.coa_microbial_cfu_text.contains("10^3") && !inp.coa_microbial_cfu_text.contains("10^4");
    if !coa_ok || inp.audio_ndt_defect_ping {
        let v = AgentVerdict::G2RejectContradictionLock;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    let verifier_required = match inp.sector {
        CbamSectorCategory::SteelAluminumCementFertilizer => inp.annual_import_net_mass_kg > 50_000,
        CbamSectorCategory::ElectricityOrHydrogen => inp.annual_import_net_mass_kg > 0,
    };
    if verifier_required && !inp.verifier_report_attached {
        let v = if inp.supplement_count == 0 {
            AgentVerdict::G3HoldSimpleReceiptMissing
        } else {
            AgentVerdict::G1ExpiredMandatoryReject
        };
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, true));
    }
    let v = AgentVerdict::G3EligibleNormal;
    (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dom06_cbam_50t_threshold_and_superscript_cfu() {
        let exempt = Dom06RaQaEsgInput {
            sector: CbamSectorCategory::SteelAluminumCementFertilizer,
            annual_import_net_mass_kg: 49_500,
            verifier_report_attached: false,
            coa_microbial_cfu_text: "1.2 x 10^3 CFU/g".into(),
            calib_sig_valid: true,
            catalyst_grade: N2sfGrade::Sensitive,
            target_zone: N2sfGrade::Sensitive,
            audio_ndt_defect_ping: false,
            supplement_count: 0,
        };
        assert_eq!(evaluate_dom06_ra_qa_esg(&exempt, OutputForm::ApprovalNotice).0, AgentVerdict::G3EligibleNormal);

        let over_50t = Dom06RaQaEsgInput { annual_import_net_mass_kg: 50_001, ..exempt.clone() };
        assert_eq!(evaluate_dom06_ra_qa_esg(&over_50t, OutputForm::ApprovalNotice).0, AgentVerdict::G3HoldSimpleReceiptMissing);

        let hydrogen = Dom06RaQaEsgInput {
            sector: CbamSectorCategory::ElectricityOrHydrogen,
            annual_import_net_mass_kg: 500,
            ..exempt.clone()
        };
        assert_eq!(evaluate_dom06_ra_qa_esg(&hydrogen, OutputForm::ApprovalNotice).0, AgentVerdict::G3HoldSimpleReceiptMissing);

        let bad_coa = Dom06RaQaEsgInput { coa_microbial_cfu_text: "1.2 x 10^4 CFU/g".into(), ..exempt };
        assert_eq!(evaluate_dom06_ra_qa_esg(&bad_coa, OutputForm::ApprovalNotice).0, AgentVerdict::G2RejectContradictionLock);
    }
}
