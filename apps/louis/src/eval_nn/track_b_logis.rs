//! Track B (`DOM-08..DOM-10`): Trade UCP 600, ISO 6346 Mod-11 bonded hazmat, and Retail SRM 2026 rules (`D9 / D11`).

use anyhow::{ensure, Result};
use chrono::NaiveDate;
use lv_agent::gates::AgentVerdict;
use lv_domain::retail_srm::{RetailContractKind, RetailSrmCheckInput};
use lv_domain::trade_logistics::{LogisticsCodeScheme, TradeDiscrepancyInput, WarehouseCheckInput};

pub const fn iso6346_letter_val(c: char) -> Option<u32> {
    match c.to_ascii_uppercase() {
        'A' => Some(10),
        'B' => Some(12), 'C' => Some(13), 'D' => Some(14), 'E' => Some(15), 'F' => Some(16),
        'G' => Some(17), 'H' => Some(18), 'I' => Some(19), 'J' => Some(20), 'K' => Some(21),
        'L' => Some(23), 'M' => Some(24), 'N' => Some(25), 'O' => Some(26), 'P' => Some(27),
        'Q' => Some(28), 'R' => Some(29), 'S' => Some(30), 'T' => Some(31), 'U' => Some(32),
        'V' => Some(34), 'W' => Some(35), 'X' => Some(36), 'Y' => Some(37), 'Z' => Some(38),
        _ => None,
    }
}

pub fn compute_iso6346_check_digit(prefix10: &[char]) -> Option<u8> {
    if prefix10.len() != 10 || !matches!(prefix10[3], 'U' | 'J' | 'Z') {
        return None;
    }
    let mut sum = 0u32;
    for (i, &ch) in prefix10.iter().enumerate() {
        let v = if i < 4 {
            iso6346_letter_val(ch)?
        } else {
            ch.to_digit(10)?
        };
        sum += v * (1u32 << i);
    }
    let rem = sum % 11;
    Some(if rem == 10 { 0 } else { rem as u8 })
}

pub fn repair_and_verify_iso6346(raw: &str) -> Option<String> {
    let cleaned: Vec<char> = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if cleaned.len() != 11 {
        return None;
    }
    let to_letter = |c: char| match c {
        '0' => 'O', '1' => 'I', '2' => 'Z', '5' => 'S', '8' => 'B', _ => c,
    };
    let to_digit = |c: char| match c {
        'O' | 'Q' => '0', 'I' | 'L' => '1', 'Z' => '2', 'S' => '5', 'G' => '6', 'B' => '8', _ => c,
    };
    let mut fixed = Vec::with_capacity(11);
    for (i, &c) in cleaned.iter().enumerate() {
        fixed.push(if i < 4 { to_letter(c) } else { to_digit(c) });
    }
    let want_cd = compute_iso6346_check_digit(&fixed[..10])?;
    let got_cd = fixed[10].to_digit(10)? as u8;
    (want_cd == got_cd).then(|| fixed.into_iter().collect())
}

pub fn synth_disguised_srm_xls_bytes(selected_state: &str, bank_acct: &str) -> Vec<u8> {
    format!(
        r#"<!DOCTYPE html><html lang="ko"><body>
        <header><nav>SRM 파트너 포털 | 공지 | 로그아웃</nav></header>
        <aside>정산관리 메뉴 트리</aside>
        <main>
          <select name="payout_hold_code">
            <option value="정상지급">정상지급</option>
            <option value="{selected_state}" selected>{selected_state}</option>
            <option value="판촉분담_재산정">판촉분담_재산정</option>
          </select>
          <table id="srm_settle_grid">
            <thead><tr><th>정산번호</th><th>품목코드</th><th>납품수량</th><th>정산금액</th><th>입금계좌</th></tr></thead>
            <tbody>
              <tr><td>SRM-202611-01</td><td>SKU-8801</td><td>500</td><td>25,000,000</td><td>{bank_acct}</td></tr>
              <tr><td>SRM-202611-02</td><td>SKU-8802</td><td>300</td><td>15,000,000</td><td>{bank_acct}</td></tr>
            </tbody>
          </table>
        </main></body></html>"#
    )
    .into_bytes()
}

pub fn evaluate_trade_discrepancy(inp: &TradeDiscrepancyInput) -> AgentVerdict {
    if inp.certificate_of_origin_missing {
        return AgentVerdict::G3HoldSimpleReceiptMissing;
    }
    if inp.ci_amount > inp.lc_limit
        || inp.ci_amount != inp.po_amount
        || inp.bl_ship_ymd > inp.lc_latest_ship_ymd
        || inp.pl_packages != inp.bl_packages
    {
        return AgentVerdict::G2RejectContradictionLock;
    }
    AgentVerdict::G3EligibleNormal
}

pub fn evaluate_warehouse_rules(inp: &WarehouseCheckInput) -> AgentVerdict {
    if !inp.bonded_release_verified {
        return AgentVerdict::G2RejectContradictionLock;
    }
    if repair_and_verify_iso6346(&inp.container_code).is_none() {
        return AgentVerdict::G2RejectContradictionLock;
    }
    if inp.un_hazmat_code.is_some() && !inp.msds_matched {
        return AgentVerdict::G2RejectContradictionLock;
    }
    if inp.un_hazmat_code.is_some() && !inp.vent_loto_complete {
        return AgentVerdict::G3InterceptInterlockOrder;
    }
    AgentVerdict::G3EligibleNormal
}

fn days_between_ymd(start_ymd: u32, end_ymd: u32) -> Result<i64> {
    let parse = |ymd: u32| -> Result<NaiveDate> {
        NaiveDate::from_ymd_opt((ymd / 10_000) as i32, (ymd / 100) % 100, ymd % 100)
            .ok_or_else(|| anyhow::anyhow!("invalid ymd: {ymd}"))
    };
    Ok((parse(end_ymd)? - parse(start_ymd)?).num_days())
}

pub fn evaluate_retail_srm_rules(
    inp: &RetailSrmCheckInput,
    master_account: &str,
    submitted_account: &str,
) -> Result<AgentVerdict> {
    if master_account.trim() != submitted_account.trim() {
        return Ok(AgentVerdict::G3ViolationHighRisk);
    }
    if inp.unilateral_return_krw > 0 && !inp.return_defect_evidence {
        return Ok(AgentVerdict::G3ViolationHighRisk);
    }
    if inp.promo_total_cost_krw > 0 {
        match (inp.promo_written_agreed_ymd, inp.promo_start_ymd) {
            (Some(agreed), Some(start)) if agreed <= start => {}
            _ => return Ok(AgentVerdict::G2RejectContradictionLock),
        }
        if inp.vendor_promo_burden_krw * 2 > inp.promo_total_cost_krw
            && !inp.voluntary_differentiated_proof
        {
            return Ok(AgentVerdict::G3ViolationHighRisk);
        }
    }
    let cap_days: i64 = match (inp.contract_kind, inp.apply_2026_amended_deadline) {
        (RetailContractKind::DirectPurchaseItemwise, false) => 60,
        (RetailContractKind::DirectPurchaseItemwise, true) => 35,
        (RetailContractKind::DirectPurchaseMonthly, _) => 20,
        (RetailContractKind::SpecialPurchase | RetailContractKind::Consignment, false) => 40,
        (RetailContractKind::SpecialPurchase | RetailContractKind::Consignment, true) => 20,
    };
    let elapsed = days_between_ymd(inp.base_close_or_receipt_ymd, inp.scheduled_payout_ymd)?;
    if elapsed < 0 || elapsed > cap_days {
        return Ok(AgentVerdict::G1ExpiredMandatoryReject);
    }
    Ok(AgentVerdict::G3EligibleNormal)
}

pub fn run_track_b_d9_and_d11_probes() -> Result<()> {
    ensure!(repair_and_verify_iso6346("MSKU43I8Z95").as_deref() == Some("MSKU4318295"));
    ensure!(repair_and_verify_iso6346("MSCU113466O").as_deref() == Some("MSCU1134660"));
    ensure!(repair_and_verify_iso6346("MSCU1234567").is_none());

    for (a, b) in [("INS", "IP"), ("CA", "COA"), ("PHYTO", "PC"), ("BC", "BK")] {
        ensure!(lv_domain::trade_logistics::trading_relay_pair(a, b).is_none());
        ensure!(lv_domain::trade_logistics::trading_relay_pair(b, a).is_none());
    }

    let trade_ok = TradeDiscrepancyInput {
        po_amount: 240_000,
        ci_amount: 240_000,
        lc_limit: 250_000,
        bl_ship_ymd: 2026_1115,
        lc_latest_ship_ymd: 2026_1120,
        pl_packages: 48,
        bl_packages: 48,
        certificate_of_origin_missing: false,
    };
    ensure!(evaluate_trade_discrepancy(&trade_ok) == AgentVerdict::G3EligibleNormal);
    ensure!(
        evaluate_trade_discrepancy(&TradeDiscrepancyInput {
            bl_ship_ymd: 2026_1121,
            ..trade_ok.clone()
        }) == AgentVerdict::G2RejectContradictionLock
    );

    let wh_ok = WarehouseCheckInput {
        container_code: "MSKU4318295".into(),
        bonded_release_verified: true,
        un_hazmat_code: Some("UN1263".into()),
        msds_matched: true,
        vent_loto_complete: true,
    };
    ensure!(evaluate_warehouse_rules(&wh_ok) == AgentVerdict::G3EligibleNormal);
    ensure!(
        evaluate_warehouse_rules(&WarehouseCheckInput {
            vent_loto_complete: false,
            ..wh_ok.clone()
        }) == AgentVerdict::G3InterceptInterlockOrder
    );

    let xls_bytes = synth_disguised_srm_xls_bytes("지급보류_검토중", "110-402-883920");
    let ir = lv_doc::docs::parse_docs_bytes("xls", &xls_bytes)?;
    ensure!(!ir.tables.is_empty());

    let base_ok = RetailSrmCheckInput {
        contract_kind: RetailContractKind::DirectPurchaseMonthly,
        apply_2026_amended_deadline: true,
        base_close_or_receipt_ymd: 2026_1130,
        scheduled_payout_ymd: 2026_1220,
        promo_total_cost_krw: 20_000_000,
        vendor_promo_burden_krw: 10_000_000,
        promo_written_agreed_ymd: Some(2026_1025),
        promo_start_ymd: Some(2026_1101),
        voluntary_differentiated_proof: false,
        unilateral_return_krw: 0,
        return_defect_evidence: false,
    };
    ensure!(
        evaluate_retail_srm_rules(&base_ok, "110-402-883920", "110-402-883920")?
            == AgentVerdict::G3EligibleNormal
    );
    ensure!(
        evaluate_retail_srm_rules(
            &RetailSrmCheckInput {
                vendor_promo_burden_krw: 10_000_001,
                ..base_ok.clone()
            },
            "110-402-883920",
            "110-402-883920"
        )? == AgentVerdict::G3ViolationHighRisk
    );
    ensure!(
        evaluate_retail_srm_rules(
            &RetailSrmCheckInput {
                scheduled_payout_ymd: 2026_1221,
                ..base_ok.clone()
            },
            "110-402-883920",
            "110-402-883920"
        )? == AgentVerdict::G1ExpiredMandatoryReject
    );
    ensure!(
        evaluate_retail_srm_rules(&base_ok, "110-402-883920", "3333-09-1122334")?
            == AgentVerdict::G3ViolationHighRisk
    );
    let _ = LogisticsCodeScheme::HsCode10;
    Ok(())
}
