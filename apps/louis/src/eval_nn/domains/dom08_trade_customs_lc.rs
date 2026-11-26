//! `DOM-08 (trade_customs_lc)`: 7-group 55+1 Trade docs, 4-Hub priority, UCP 600 / ISBP 821 / ATA Carnet,
//! self-loop-free `trading_relay_pair_v2`, HSK-10 repair, 4-script phonetic alias gate.

use lv_agent::gates::{select_form_for_verdict, AgentVerdict, FormChoice, OutputForm};

pub const DOM08_FORMS: [&str; 6] = [
    "DOM08_F01_CONTRACT_LC",
    "DOM08_F02_SHIPPING_BL_CI",
    "DOM08_F03_CUSTOMS_HSK_ATA",
    "DOM08_F04_INSPECT_IC_COA",
    "DOM08_F05_INSURE_DGD_LEGAL",
    "DOM08_F06_SETTLE_TRACK_AUDIO",
];

pub const TRADE_GROUPS_56: [(&str, &[&str]); 7] = [
    ("contract", &["PO", "PI", "SC", "LC", "LLC", "CP", "BE", "TR", "LG", "EL"]),
    ("shipping", &["CI", "PL", "BL", "HBL", "SWB", "AWB", "SA", "DO", "AN", "BC", "BK", "SR", "FCR", "POD", "CM", "FI", "WR"]),
    ("customs", &["ED", "ID", "CINV", "CO", "CCC", "CNM", "CSI"]),
    ("inspection", &["IC", "WC", "CA", "COA", "PHYTO", "PC", "HC", "BEN_CERT", "FC", "CDR"]),
    ("legal", &["DGD", "MSDS", "POA", "BIZ_LIC", "INS", "IP", "ICF"]),
    ("settlement", &["SOA", "DN", "CN", "TI"]),
    ("parcel", &["TRACKING"]),
];

pub const TRADE_HUB_PRIORITY: [(&str, &str); 4] = [
    ("PO", "reference_po"),
    ("CI", "reference_invoice"),
    ("BL", "reference_bl"),
    ("LC", "reference_lc"),
];

pub fn normalize_trade_identifier(raw: &str) -> String {
    raw.chars()
        .filter_map(|c| {
            let mapped = match c {
                '\u{FF10}'..='\u{FF19}' => char::from_u32(c as u32 - 0xFF10 + '0' as u32)?,
                '\u{FF21}'..='\u{FF3A}' => char::from_u32(c as u32 - 0xFF21 + 'A' as u32)?,
                '\u{FF41}'..='\u{FF5A}' => char::from_u32(c as u32 - 0xFF41 + 'A' as u32)?,
                other => other,
            };
            mapped.is_alphanumeric().then(|| mapped.to_ascii_uppercase())
        })
        .collect()
}

pub fn trade_reference_field_of(doc_type: &str) -> Option<&'static str> {
    match doc_type.trim().to_ascii_uppercase().as_str() {
        "PO" => Some("reference_po"),
        "LC" => Some("reference_lc"),
        "CI" => Some("reference_invoice"),
        "PL" => Some("reference_packing"),
        "BL" => Some("reference_bl"),
        "BC" | "BK" => Some("reference_booking"),
        "CO" => Some("reference_origin"),
        "CNM" => Some("reference_non_manipulation"),
        "CA" | "COA" => Some("reference_analysis"),
        "PHYTO" | "PC" => Some("reference_phyto"),
        "INS" | "IP" => Some("reference_policy"),
        _ => None,
    }
}

/// Blocks alias self-loops (`INS<->IP`, `CA<->COA`, `PHYTO<->PC`, `BC<->BK`) where `mine == foreign`.
pub fn trading_relay_pair_v2(from_type: &str, to_type: &str) -> Option<(&'static str, &'static str)> {
    if from_type.eq_ignore_ascii_case(to_type) {
        return None;
    }
    let mine = trade_reference_field_of(to_type)?;
    let foreign = trade_reference_field_of(from_type)?;
    (mine != foreign).then_some((mine, foreign))
}

pub fn repair_and_verify_hsk10(raw: &str) -> Option<String> {
    let digits: String = raw
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '.' | '-' | '_' | '/'))
        .map(|c| match c.to_ascii_uppercase() {
            'O' | 'Q' => '0',
            'I' | 'L' => '1',
            'Z' => '2',
            'S' => '5',
            'G' => '6',
            'B' => '8',
            other => other,
        })
        .collect();
    if digits.len() != 10 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let ch: u8 = digits[0..2].parse().ok()?;
    ((1..=97).contains(&ch) && ch != 77).then_some(digits)
}

/// Evaluates phonetic alias safety across Ko/Ja/Zh/Ar scripts: requires skeleton length >= 4,
/// single target script purity (rejects mixed scripts), and rejects letter-by-letter spelling noise.
pub fn gate_native_alias(canonical_port: &str, candidate_alias: &str) -> bool {
    let alias = candidate_alias.trim();
    if canonical_port.is_empty() || alias.is_empty() {
        return false;
    }
    let has_ascii = alias.chars().any(|c| c.is_ascii_alphabetic());
    let has_ko = alias.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c));
    let has_ja = alias
        .chars()
        .any(|c| ('\u{3040}'..='\u{30FF}').contains(&c) || c == 'ー');
    let has_zh = alias.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c));
    let has_ar = alias.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c));
    let scripts = [has_ascii, has_ko, has_ja, has_zh, has_ar]
        .into_iter()
        .filter(|&b| b)
        .count();
    if scripts != 1 || has_ascii {
        return false;
    }
    let skeleton_len = alias.chars().filter(|c| !c.is_whitespace() && *c != '・').count();
    if skeleton_len < 4 {
        return false;
    }
    // Reject Korean letter-spelling artifacts ("에프이엘아이엑스...")
    const SPELLED_PREFIXES: &[&str] = &["에프", "이엘", "에이치", "더블유"];
    if SPELLED_PREFIXES.iter().any(|p| alias.contains(p)) {
        return false;
    }
    true
}

#[derive(Debug, Clone)]
pub struct Dom08TradeCustomsInput {
    pub po_amount: i64,
    pub ci_amount: i64,
    pub lc_limit: i64,
    pub ship_ymd: u32,
    pub lc_latest_ship_ymd: u32,
    pub bl_on_board_notated: bool,
    pub fta_origin_missing_or_cnm_missing: bool,
    pub inspect_cert_after_ship_allowed_isbp821: bool,
    pub inspect_issue_ymd: u32,
    pub requires_preshipment_clause: bool,
    pub cif_or_cip_insured_bps: u32,
    pub insurance_effective_ymd: u32,
    pub ata_carnet_contains_consumables_or_un0431: bool,
    pub hsk10_raw: String,
    pub supplement_count: u32,
}

pub fn evaluate_dom08_trade_customs(
    inp: &Dom08TradeCustomsInput,
    requested_form: OutputForm,
) -> (AgentVerdict, FormChoice) {
    if inp.ata_carnet_contains_consumables_or_un0431 {
        let v = AgentVerdict::G1ExpiredMandatoryReject;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, false));
    }
    if inp.fta_origin_missing_or_cnm_missing {
        let v = AgentVerdict::G3HoldSimpleReceiptMissing;
        return (v, select_form_for_verdict(requested_form, v, inp.supplement_count, true));
    }
    let inspect_violation = inp.requires_preshipment_clause && inp.inspect_issue_ymd > inp.ship_ymd;
    let ins_violation = inp.cif_or_cip_insured_bps < 11_000 || inp.insurance_effective_ymd > inp.ship_ymd;
    if !inp.bl_on_board_notated
        || inp.ci_amount > inp.lc_limit
        || inp.ci_amount != inp.po_amount
        || inp.ship_ymd > inp.lc_latest_ship_ymd
        || repair_and_verify_hsk10(&inp.hsk10_raw).is_none()
        || inspect_violation
        || ins_violation
        || (!inp.inspect_cert_after_ship_allowed_isbp821 && inp.inspect_issue_ymd > inp.ship_ymd)
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
    fn dom08_groups_relay_self_loop_isbp821_ata_and_phonetic_gates() {
        let total_codes: usize = TRADE_GROUPS_56.iter().map(|(_, list)| list.len()).sum();
        assert_eq!(total_codes, 56);
        for (a, b) in [("INS", "IP"), ("CA", "COA"), ("PHYTO", "PC"), ("BC", "BK")] {
            assert!(trading_relay_pair_v2(a, b).is_none());
            assert!(trading_relay_pair_v2(b, a).is_none());
        }
        assert_eq!(trading_relay_pair_v2("PO", "CI"), Some(("reference_invoice", "reference_po")));
        assert_eq!(repair_and_verify_hsk10("847I.3O-OOOI").as_deref(), Some("8471300001"));
        assert!(gate_native_alias("Felixstowe", "펠릭스토우"));
        assert!(gate_native_alias("Felixstowe", "フェリクストウ"));
        assert!(gate_native_alias("Felixstowe", "费利克斯托"));
        assert!(gate_native_alias("Felixstowe", "فيليكستو"));
        assert!(!gate_native_alias("Felixstowe", "운송/Incheon"));
        assert!(!gate_native_alias("Felixstowe", "에프이엘아이엑스"));

        let ok = Dom08TradeCustomsInput {
            po_amount: 500_000,
            ci_amount: 500_000,
            lc_limit: 550_000,
            ship_ymd: 2026_1110,
            lc_latest_ship_ymd: 2026_1115,
            bl_on_board_notated: true,
            fta_origin_missing_or_cnm_missing: false,
            inspect_cert_after_ship_allowed_isbp821: true,
            inspect_issue_ymd: 2026_1112,
            requires_preshipment_clause: false,
            cif_or_cip_insured_bps: 11_000,
            insurance_effective_ymd: 2026_1110,
            ata_carnet_contains_consumables_or_un0431: false,
            hsk10_raw: "8542.31-1000".into(),
            supplement_count: 0,
        };
        assert_eq!(evaluate_dom08_trade_customs(&ok, OutputForm::ApprovalNotice).0, AgentVerdict::G3EligibleNormal);
        assert_eq!(
            evaluate_dom08_trade_customs(&Dom08TradeCustomsInput { ata_carnet_contains_consumables_or_un0431: true, ..ok }, OutputForm::ApprovalNotice).0,
            AgentVerdict::G1ExpiredMandatoryReject
        );
    }
}
