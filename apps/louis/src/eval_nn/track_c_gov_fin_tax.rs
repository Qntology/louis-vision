//! Track C (`DOM-11, 12, 13, 18`): Public G2B costing, financial KYC/accountability, NTS tax XML, and single-quote controls (`D12` part 1).

use anyhow::{ensure, Result};
use lv_agent::gates::{officer_approve, AgentVerdict};
use lv_domain::gov_fin_tax_audit::{
    AuditGovCheckInput, CostingRuleKind, FinKycLoanCheckInput, PublicG2bCheckInput,
    TaxEvidCheckInput,
};

pub fn verify_biz_reg_no_kr10(raw: &str) -> Option<String> {
    const WEIGHTS: [u32; 9] = [1, 3, 7, 1, 3, 7, 1, 3, 5];
    let digits: Vec<u32> = raw
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| match c {
            'O' | 'o' => Some(0),
            'I' | 'l' => Some(1),
            'B' => Some(8),
            d => d.to_digit(10),
        })
        .collect::<Option<Vec<u32>>>()?;
    if digits.len() != 10 {
        return None;
    }
    let mut sum: u32 = digits[..9]
        .iter()
        .zip(WEIGHTS.iter())
        .map(|(&d, &w)| d * w)
        .sum();
    sum += (digits[8] * 5) / 10;
    let check = (10 - (sum % 10)) % 10;
    (check == digits[9]).then(|| {
        digits
            .into_iter()
            .map(|d| char::from(b'0' + d as u8))
            .collect()
    })
}

pub fn verify_etax_approval_no_24(raw: &str) -> Option<String> {
    let s: String = raw
        .chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .collect();
    if s.len() != 24 {
        return None;
    }
    let (ymd8, rest) = s.split_at(8);
    let (cert8, serial8) = rest.split_at(8);
    if !ymd8.chars().all(|c| c.is_ascii_digit()) || !cert8.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !matches!(&cert8[..2], "10" | "20" | "41" | "42" | "50" | "70" | "90") {
        return None;
    }
    if !serial8
        .chars()
        .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase())
    {
        return None;
    }
    Some(s)
}

pub fn synth_nts_etax_xml_bytes(
    issue_id_24: &str,
    issue_ymd: u32,
    supplier_biz_no: &str,
    buyer_biz_no: &str,
    charge_krw: i64,
    tax_krw: i64,
) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<TaxInvoice xmlns="urn:kr:or:kec:standard:Tax:ReusableAggregateBusinessInformationEntitySchemaModule:1:0">
  <ExchangedDocument><ID>{issue_id_24}</ID><IssueDateTime>{issue_ymd}</IssueDateTime></ExchangedDocument>
  <TaxInvoiceDocument><IssueID>{issue_id_24}</IssueID><TypeCode>0101</TypeCode><IssueDateTime>{issue_ymd}</IssueDateTime></TaxInvoiceDocument>
  <TaxInvoiceTradeSettlement>
    <InvoicerParty><ID>{supplier_biz_no}</ID></InvoicerParty>
    <InvoiceeParty><ID>{buyer_biz_no}</ID></InvoiceeParty>
    <SpecifiedMonetarySummation>
      <ChargeTotalAmount>{charge_krw}</ChargeTotalAmount>
      <TaxTotalAmount>{tax_krw}</TaxTotalAmount>
      <GrandTotalAmount>{}</GrandTotalAmount>
    </SpecifiedMonetarySummation>
  </TaxInvoiceTradeSettlement>
</TaxInvoice>"#,
        charge_krw + tax_krw
    )
    .into_bytes()
}

pub fn evaluate_public_g2b(inp: &PublicG2bCheckInput) -> AgentVerdict {
    if inp.missing_rfp_mandatory_items > 0 {
        return AgentVerdict::G3HoldSimpleReceiptMissing;
    }
    let admin_base = (inp.direct_material_krw as i128)
        + (inp.labor_and_expense_krw as i128)
        + (inp.subcontract_processing_krw as i128)
        + (inp.tech_royalty_krw as i128);
    let admin_max = (admin_base * (inp.general_admin_cap_bps as i128)) / 10_000;
    if (inp.general_admin_claimed_krw as i128) > admin_max {
        return AgentVerdict::G2RejectContradictionLock;
    }
    let profit_base: i128 = match inp.rule_kind {
        CostingRuleKind::PublicConstructionCost | CostingRuleKind::DefenseAcquisitionCost => {
            (inp.labor_and_expense_krw as i128) + (inp.general_admin_claimed_krw as i128)
        }
        CostingRuleKind::PublicStandardMarketUnit => {
            (inp.direct_material_krw as i128)
                + (inp.labor_and_expense_krw as i128)
                + (inp.general_admin_claimed_krw as i128)
        }
    };
    let profit_max = (profit_base * (inp.profit_cap_bps as i128)) / 10_000;
    if (inp.profit_claimed_krw as i128) > profit_max {
        return AgentVerdict::G2RejectContradictionLock;
    }
    AgentVerdict::G3EligibleNormal
}

pub fn evaluate_fin_kyc_loan(inp: &FinKycLoanCheckInput) -> AgentVerdict {
    let has_accountability = inp
        .accountability_code
        .as_deref()
        .is_some_and(|c| !c.trim().is_empty());
    if !has_accountability || !inp.control_action_logged {
        return AgentVerdict::G1InterceptForceHold;
    }
    let ubo_ok =
        inp.ubo_25pct_natural_person_verified || inp.fallback_controlling_person_verified;
    if !ubo_ok
        || !inp.registry_vs_shareholder_sum_match
        || (inp.high_risk_edd_triggered && !inp.edd_enhanced_docs_attached)
    {
        return AgentVerdict::G3ViolationHighRisk;
    }
    AgentVerdict::G3EligibleNormal
}

pub const fn next_month_10th_ymd(issue_ymd: u32) -> u32 {
    let y = issue_ymd / 10_000;
    let m = (issue_ymd / 100) % 100;
    if m == 12 {
        (y + 1) * 10_000 + 110
    } else {
        y * 10_000 + (m + 1) * 100 + 10
    }
}

pub fn evaluate_tax_evid(inp: &TaxEvidCheckInput) -> AgentVerdict {
    let deadline_ymd = next_month_10th_ymd(inp.etax_issue_ymd);
    match inp.etax_nts_transmit_ymd {
        Some(tx_ymd) if tx_ymd <= deadline_ymd => {}
        _ => return AgentVerdict::G2RejectContradictionLock,
    }
    if inp.promo_expense_krw > 0 {
        if inp.vat_input_credit_claimed
            || (inp.is_traditional_market_or_local_voucher && inp.is_entertainment_excluded_venue)
        {
            return AgentVerdict::G2RejectContradictionLock;
        }
        let proof_threshold = if inp.is_condolence_expense {
            200_000
        } else {
            30_000
        };
        if inp.promo_expense_krw > proof_threshold && !inp.paid_by_corporate_qualified_proof {
            return AgentVerdict::G2RejectContradictionLock;
        }
    }
    AgentVerdict::G3EligibleNormal
}

pub fn evaluate_audit_gov(inp: &AuditGovCheckInput) -> AgentVerdict {
    if inp.is_sme_restricted_bid_but_mid_corp {
        return AgentVerdict::G3ViolationHighRisk;
    }
    if let Some(board_cap) = inp.board_approved_cap_krw {
        if inp.cumulative_executed_krw > board_cap {
            return AgentVerdict::G3ViolationHighRisk;
        }
    }
    let per_contract_cap = if inp.is_women_disabled_youth_cert {
        50_000_000
    } else {
        20_000_000
    };
    if inp.single_quote_contract_krw > per_contract_cap
        || inp.same_vendor_annual_count > inp.annual_count_cap
        || inp.same_vendor_annual_sum_krw > inp.annual_sum_cap_krw
    {
        return AgentVerdict::G3ViolationRelatedVendorSplit;
    }
    AgentVerdict::G3EligibleNormal
}

pub fn run_track_c_d12_part1_probes() -> Result<()> {
    ensure!(verify_biz_reg_no_kr10("11O-8I-41272").as_deref() == Some("1108141272"));
    ensure!(verify_etax_approval_no_24("20261031-41000012-a9b8c7d6").is_some());

    let xml = synth_nts_etax_xml_bytes("2026103141000012a9b8c7d6", 2026_1031, "1108141272", "2208100001", 1_000_000, 100_000);
    let parsed_xml = lv_doc::docs::parse_docs_bytes("xml", &xml)?;
    ensure!(parsed_xml.plain_text.contains("TaxInvoice"));

    let g2b_ok = PublicG2bCheckInput {
        rule_kind: CostingRuleKind::PublicConstructionCost,
        direct_material_krw: 400_000_000,
        gov_supplied_material_krw: 100_000_000,
        subcontract_processing_krw: 50_000_000,
        tech_royalty_krw: 10_000_000,
        labor_and_expense_krw: 540_000_000,
        general_admin_claimed_krw: 80_000_000,
        general_admin_cap_bps: 800,
        profit_claimed_krw: 93_000_000,
        profit_cap_bps: 1_500,
        missing_rfp_mandatory_items: 0,
    };
    ensure!(evaluate_public_g2b(&g2b_ok) == AgentVerdict::G3EligibleNormal);
    ensure!(
        evaluate_public_g2b(&PublicG2bCheckInput {
            profit_claimed_krw: 93_000_001,
            ..g2b_ok
        }) == AgentVerdict::G2RejectContradictionLock
    );

    let kyc_ok = FinKycLoanCheckInput {
        accountability_code: Some("ACC-CFO-02".into()),
        control_action_logged: true,
        ubo_25pct_natural_person_verified: true,
        fallback_controlling_person_verified: false,
        registry_vs_shareholder_sum_match: true,
        high_risk_edd_triggered: false,
        edd_enhanced_docs_attached: false,
    };
    ensure!(evaluate_fin_kyc_loan(&kyc_ok) == AgentVerdict::G3EligibleNormal);
    ensure!(
        evaluate_fin_kyc_loan(&FinKycLoanCheckInput {
            accountability_code: None,
            ..kyc_ok.clone()
        }) == AgentVerdict::G1InterceptForceHold
    );

    let tax_ok = TaxEvidCheckInput {
        etax_issue_ymd: 2026_1031,
        etax_nts_transmit_ymd: Some(2026_1110),
        promo_expense_krw: 30_000,
        vat_input_credit_claimed: false,
        is_traditional_market_or_local_voucher: false,
        is_entertainment_excluded_venue: false,
        is_condolence_expense: false,
        paid_by_corporate_qualified_proof: false,
    };
    ensure!(evaluate_tax_evid(&tax_ok) == AgentVerdict::G3EligibleNormal);
    ensure!(
        evaluate_tax_evid(&TaxEvidCheckInput {
            promo_expense_krw: 30_001,
            ..tax_ok.clone()
        }) == AgentVerdict::G2RejectContradictionLock
    );

    let gov_split = AuditGovCheckInput {
        single_quote_contract_krw: 18_500_000,
        is_women_disabled_youth_cert: false,
        is_sme_restricted_bid_but_mid_corp: false,
        same_vendor_annual_count: 6,
        same_vendor_annual_sum_krw: 110_500_000,
        annual_count_cap: 5,
        annual_sum_cap_krw: 200_000_000,
        board_approved_cap_krw: Some(1_000_000_000),
        cumulative_executed_krw: 980_000_000,
    };
    let v_split = evaluate_audit_gov(&gov_split);
    ensure!(v_split == AgentVerdict::G3ViolationRelatedVendorSplit);
    ensure!(officer_approve(v_split, None) == Err(AgentVerdict::G3InterceptRubberStamp));
    Ok(())
}
