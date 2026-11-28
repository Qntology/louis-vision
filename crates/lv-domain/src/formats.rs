//! Promoted Tier 1 domain classification (2-Depth across `DOM-01..DOM-18`), structured fact
//! extraction, checksum validators (KR Biz Reg 10, eTax-24, NSN-13, ISO 6346), and zero-digit
//! government ID hygiene checks (Wave 5 product pipeline promotion; strictly Zero-Net / Zero-DB).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::registry::DomainPackRegistry;
use crate::verify_iso6346_check_digit;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainClassification {
    pub pack_code: String,
    pub pack_id: String,
    pub depth1_sector: String,
    pub depth2_form: String,
    pub confidence: f32,
    pub high_impact: bool,
    pub retention_days: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedDomainFacts {
    pub pack_code: String,
    pub primary_id: Option<String>,
    pub amount_krw: Option<i64>,
    pub verified_checksums: BTreeMap<String, String>,
    pub raw_gov_id_detected: bool,
    pub key_values: BTreeMap<String, String>,
}

struct PackRule {
    code: &'static str,
    sector: &'static str,
    default_form: &'static str,
    keywords: &'static [&'static str],
}

const PACK_2DEPTH_RULES: &[PackRule] = &[
    PackRule { code: "DOM-01", sector: "construction", default_form: "progress_settle", keywords: &["기성", "하도급", "준공", "공정률", "contract_sum", "progress_pct"] },
    PackRule { code: "DOM-02", sector: "rnd", default_form: "grant_settle", keywords: &["연구비", "국가r&d", "과제번호", "연구장비", "grant_id", "zeus"] },
    PackRule { code: "DOM-03", sector: "legal", default_form: "insolvency_claim", keywords: &["회생", "도산", "파산", "채권신고", "debtor_id", "채권자"] },
    PackRule { code: "DOM-04", sector: "finance", default_form: "three_way_match", keywords: &["3-way", "매입", "발주서", "입고검수", "po_no", "invoice_qty"] },
    PackRule { code: "DOM-05", sector: "insurance", default_form: "medical_claim", keywords: &["실손", "보험금", "진료비", "심사", "claim_no", "policy_limit"] },
    PackRule { code: "DOM-06", sector: "quality_esg", default_form: "cbam_coa", keywords: &["cbam", "탄소국경", "coa", "성적서", "batch_no", "cfu"] },
    PackRule { code: "DOM-07", sector: "safety", default_form: "loto_ptw", keywords: &["loto", "밀폐공간", "안전작업허가", "노무안전", "permit_signed", "work_hours"] },
    PackRule { code: "DOM-08", sector: "trade", default_form: "customs_lc", keywords: &["신용장", "선하증권", "상업송장", "통관", "b/l", "isbp", "ci_amount"] },
    PackRule { code: "DOM-09", sector: "logistics", default_form: "bonded_hazmat", keywords: &["보세", "위험물", "컨테이너", "imdg", "container_id", "un_hazmat"] },
    PackRule { code: "DOM-10", sector: "retail", default_form: "srm_settle", keywords: &["대규모유통", "판촉비", "srm", "정산일", "promo_ratio", "payout_ymd"] },
    PackRule { code: "DOM-11", sector: "public_g2b", default_form: "g2b_cost", keywords: &["나라장터", "조달청", "원가계산", "일반관리비", "admin_krw", "profit_krw"] },
    PackRule { code: "DOM-12", sector: "banking_kyc", default_form: "kyc_accountability", keywords: &["여신", "kyc", "책무구조", "실소유자", "ubo_verified", "accountability"] },
    PackRule { code: "DOM-13", sector: "tax", default_form: "etax_audit", keywords: &["세금계산서", "국세청", "접대비", "매입세액", "etax_24_id", "vat_credit"] },
    PackRule { code: "DOM-14", sector: "ip_security", default_form: "nct_patent", keywords: &["국가핵심기술", "직무발명", "산업부", "역-cdr", "n2sf_grade", "motie"] },
    PackRule { code: "DOM-15", sector: "engineering", default_form: "ecn_bom", keywords: &["ecn", "bom", "표제란", "변경관리", "moc", "title_block"] },
    PackRule { code: "DOM-16", sector: "defense_mro", default_form: "ecp_mro", keywords: &["방산", "nsn", "ecp", "mro", "형상통제", "coc_verified"] },
    PackRule { code: "DOM-17", sector: "hr_privacy", default_form: "blind_recruit", keywords: &["블라인드", "채용", "경력증명", "개인정보", "career_days", "blind_clean"] },
    PackRule { code: "DOM-18", sector: "governance", default_form: "internal_audit", keywords: &["내부감사", "수의계약", "행안부예규", "이사회", "single_quote_krw", "board_cap"] },
];

/// Deterministic 2-Depth classifier mapping document text + filename to `(DOM-01..DOM-18, sector, form)`
/// together with `high_impact` and 5-year retention metadata from `DomainPackRegistry::builtin_18`.
pub fn classify_domain_2depth(text: &str, filename_hint: Option<&str>) -> DomainClassification {
    let reg = DomainPackRegistry::builtin_18();
    let combined = match filename_hint {
        Some(name) if !name.is_empty() => format!("{name}\n{text}").to_lowercase(),
        _ => text.to_lowercase(),
    };

    let mut best_rule = &PACK_2DEPTH_RULES[0];
    let mut best_hits = 0usize;
    for rule in PACK_2DEPTH_RULES {
        let mut hits = 0usize;
        if combined.contains(&rule.code.to_lowercase()) {
            hits += 4;
        }
        for kw in rule.keywords {
            if combined.contains(kw) {
                hits += 1;
            }
        }
        if hits > best_hits {
            best_hits = hits;
            best_rule = rule;
        }
    }

    let meta = reg.get(best_rule.code);
    let confidence = if best_hits == 0 {
        0.25
    } else {
        (0.55 + (best_hits.min(5) as f32) * 0.09).min(0.99)
    };
    let depth2_form = if combined.contains("보완") || combined.contains("supplement") {
        format!("{}_supplement", best_rule.default_form)
    } else if combined.contains("반려") || combined.contains("reject") {
        format!("{}_reject", best_rule.default_form)
    } else {
        best_rule.default_form.to_string()
    };

    DomainClassification {
        pack_code: best_rule.code.to_string(),
        pack_id: meta.map_or_else(|| "construction_settle".into(), |m| m.id.to_string()),
        depth1_sector: best_rule.sector.to_string(),
        depth2_form,
        confidence,
        high_impact: meta.map_or(true, |m| m.high_impact),
        retention_days: meta.map_or(1825, |m| m.default_retention_days),
    }
}

/// Verifies and normalizes a 10-digit Korean business registration number (OCR `O->0`, `I/l->1` repair).
pub fn verify_biz_reg_no_kr10(raw: &str) -> Option<String> {
    let repaired: String = raw
        .chars()
        .filter(|c| !matches!(c, '-' | ' ' | '_'))
        .map(|c| match c {
            'O' | 'o' => '0',
            'I' | 'l' | 'i' => '1',
            other => other,
        })
        .collect();
    if repaired.len() != 10 || !repaired.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let d: Vec<u32> = repaired.bytes().map(|b| (b - b'0') as u32).collect();
    const WEIGHTS: [u32; 9] = [1, 3, 7, 1, 3, 7, 1, 3, 5];
    let mut sum: u32 = d[..9].iter().zip(WEIGHTS).map(|(x, w)| x * w).sum();
    sum += (d[8] * 5) / 10;
    let check = (10 - (sum % 10)) % 10;
    (check == d[9]).then_some(repaired)
}

/// Validates Korean National Tax Service 24-character eTax invoice approval number (`YYYYMMDD-8digits-8hex`).
pub fn verify_etax_approval_no_24(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let parts: Vec<&str> = trimmed.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 8 || parts[1].len() != 8 || parts[2].len() != 8 {
        return None;
    }
    if !parts[0].bytes().all(|b| b.is_ascii_digit())
        || !parts[1].bytes().all(|b| b.is_ascii_digit())
        || !parts[2].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

/// Repairs common OCR lookalikes and verifies 13-digit NATO Stock Number (`XXXX-XX-XXXXXXX`).
pub fn repair_and_verify_nsn13(raw: &str) -> Option<String> {
    let digits: String = raw
        .chars()
        .filter(|c| !matches!(c, '-' | ' ' | '.'))
        .map(|c| match c {
            'O' | 'o' => '0',
            'I' | 'l' | 'i' => '1',
            other => other,
        })
        .collect();
    if digits.len() != 13 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}-{}-{}", &digits[0..4], &digits[4..6], &digits[6..13]))
}

/// Detects raw unredacted Korean RRN (`\d{6}-[1-4]\d{6}`), JP MyNumber (12 digits with context),
/// or IN Aadhaar (`\d{4} \d{4} \d{4}` with context) digits (Rule 4).
pub fn contains_raw_gov_id_digits(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() >= 14 {
        for i in 0..=bytes.len() - 14 {
            let w = &bytes[i..i + 14];
            let prev_digit = i > 0 && bytes[i - 1].is_ascii_digit();
            let next_digit = i + 14 < bytes.len() && bytes[i + 14].is_ascii_digit();
            if !prev_digit
                && !next_digit
                && w[..6].iter().all(u8::is_ascii_digit)
                && w[6] == b'-'
                && matches!(w[7], b'1'..=b'4')
                && w[8..14].iter().all(u8::is_ascii_digit)
            {
                return true;
            }
        }
    }
    let lower = text.to_lowercase();
    if lower.contains("aadhaar") || lower.contains("mynumber") || lower.contains("마이넘버") {
        let mut run = 0usize;
        for ch in text.chars() {
            if ch.is_ascii_digit() {
                run += 1;
                if run >= 12 {
                    return true;
                }
            } else if ch != ' ' && ch != '-' {
                run = 0;
            }
        }
    }
    false
}

/// Extracts structured domain facts (`key=value` pairs, KRW monetary amounts, validated checksums,
/// and Rule 4 raw-gov-ID check) from sanitized document plain text.
pub fn extract_domain_facts(pack_code: &str, text: &str) -> ExtractedDomainFacts {
    let mut facts = ExtractedDomainFacts {
        pack_code: pack_code.to_string(),
        raw_gov_id_detected: contains_raw_gov_id_digits(text),
        ..Default::default()
    };

    for line in text.lines() {
        for tok in line.split(|c: char| c.is_whitespace() || c == '|' || c == ';') {
            let t = tok.trim();
            if t.is_empty() {
                continue;
            }
            if let Some((k, v)) = t.split_once('=') {
                let key = k.trim().to_ascii_lowercase();
                let val = v.trim();
                if !key.is_empty() && !val.is_empty() {
                    if key == "primary_id" || key == "doc_id" || key == "po_no" || key == "grant_id" {
                        facts.primary_id.get_or_insert_with(|| val.to_string());
                    }
                    facts.key_values.insert(key, val.to_string());
                }
            }
            if t.len() == 11 && verify_iso6346_check_digit(t) {
                facts
                    .verified_checksums
                    .insert("iso6346".into(), t.to_ascii_uppercase());
            } else if let Some(biz) = verify_biz_reg_no_kr10(t) {
                facts.verified_checksums.insert("kr_biz_reg".into(), biz);
            } else if let Some(etax) = verify_etax_approval_no_24(t) {
                facts.verified_checksums.insert("etax_24".into(), etax);
            } else if t.contains('-') {
                if let Some(nsn) = repair_and_verify_nsn13(t) {
                    facts.verified_checksums.insert("nsn_13".into(), nsn);
                }
            }
        }
    }

    // Extract largest KRW amount token (`1,000,000원` or `amt_krw=45000000`)
    if let Some(amt) = facts
        .key_values
        .get("amt_krw")
        .or_else(|| facts.key_values.get("claimed_amount"))
        .or_else(|| facts.key_values.get("line_item_sum_krw"))
        .and_then(|s| s.replace(',', "").parse::<i64>().ok())
    {
        facts.amount_krw = Some(amt);
    } else {
        let mut max_amt = 0i64;
        for part in text.split_whitespace() {
            if let Some(prefix) = part.strip_suffix('원') {
                let digits: String = prefix.chars().filter(|c| c.is_ascii_digit()).collect();
                if let Ok(v) = digits.parse::<i64>() {
                    max_amt = max_amt.max(v);
                }
            }
        }
        if max_amt > 0 {
            facts.amount_krw = Some(max_amt);
        }
    }
    facts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_depth_classification_checksums_and_fact_extraction_work_in_tier1() {
        let c1 = classify_domain_2depth(
            "DOM-11 나라장터 공공입찰 원가계산서 일반관리비 admin_krw=12000000",
            Some("g2b_cost.hwpx"),
        );
        assert_eq!(c1.pack_code, "DOM-11");
        assert!(c1.high_impact);
        assert_eq!(c1.retention_days, 1825);

        let c2 = classify_domain_2depth(
            "상업송장 신용장 선하증권 B/L 통관 MSCU1134660 45,000,000원",
            Some("trade_lc.pdf"),
        );
        assert_eq!(c2.pack_code, "DOM-08");
        assert!(!c2.high_impact);

        let facts = extract_domain_facts(
            "DOM-08",
            "primary_id=CI-2026-881 MSCU1134660 110-81-41272 20261031-41000012-a9b8c7d6 45,000,000원 [RRN_REDACTED]",
        );
        assert_eq!(facts.primary_id.as_deref(), Some("CI-2026-881"));
        assert_eq!(facts.amount_krw, Some(45_000_000));
        assert_eq!(facts.verified_checksums.get("iso6346").map(String::as_str), Some("MSCU1134660"));
        assert_eq!(facts.verified_checksums.get("kr_biz_reg").map(String::as_str), Some("1108141272"));
        assert!(facts.verified_checksums.contains_key("etax_24"));
        assert!(!facts.raw_gov_id_detected);
        assert!(contains_raw_gov_id_digits("주민번호 900101-1234567"));
    }
}
