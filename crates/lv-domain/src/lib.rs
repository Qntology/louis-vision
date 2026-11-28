//! Domain modes & packs for `louis-vision` (Layer 3):
//! - 7 Air-Gapped Docs Domain Packs (`construction_settle`, `rnd_audit`, `insolvency_legal`,
//!   `finance_3way`, `claims_med`, `ra_qa_esg`, `safety_hr`) with `LOUIS_ENGINE=rules` fast path
//!   and offline `premium-discount-eval` pack self-scoring.
//! - Industrial (`PowerPlant` KKS <-> `AiDataCenter` DCIM) `Terms` & `CodeScheme` repair.

use anyhow::Result;
pub use lv_sds::N2sfGrade;
use serde::{Deserialize, Serialize};

pub mod bridge;
pub mod formats;
pub mod gov_fin_tax_audit;
pub mod identify;
pub mod registry;
pub mod retail_srm;
pub mod sec_eng_mro_hr;
pub mod trade_logistics;

pub const ENGINE_ENV: &str = "LOUIS_ENGINE";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineMode {
    #[default]
    FullModel,
    RulesOnly,
}

impl EngineMode {
    pub fn from_env() -> Self {
        match std::env::var(ENGINE_ENV).ok().as_deref().map(str::trim) {
            Some("rules" | "rules_only") => Self::RulesOnly,
            _ => Self::FullModel,
        }
    }

    pub fn from_str_flag(raw: Option<&str>) -> Self {
        match raw.map(str::trim) {
            Some("rules" | "rules_only") => Self::RulesOnly,
            _ => Self::FullModel,
        }
    }
}

/// Seven core air-gapped docs workflow domain packs shipped with `louis-docs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocsDomainPack {
    ConstructionSettle,
    RndAudit,
    InsolvencyLegal,
    Finance3Way,
    ClaimsMed,
    RaQaEsg,
    SafetyHr,
}

impl DocsDomainPack {
    pub const ALL: [Self; 7] = [
        Self::ConstructionSettle,
        Self::RndAudit,
        Self::InsolvencyLegal,
        Self::Finance3Way,
        Self::ClaimsMed,
        Self::RaQaEsg,
        Self::SafetyHr,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::ConstructionSettle => "construction_settle",
            Self::RndAudit => "rnd_audit",
            Self::InsolvencyLegal => "insolvency_legal",
            Self::Finance3Way => "finance_3way",
            Self::ClaimsMed => "claims_med",
            Self::RaQaEsg => "ra_qa_esg",
            Self::SafetyHr => "safety_hr",
        }
    }

    pub fn code_prefix(self) -> &'static str {
        match self {
            Self::ConstructionSettle => "DOM-01",
            Self::RndAudit => "DOM-02",
            Self::InsolvencyLegal => "DOM-03",
            Self::Finance3Way => "DOM-04",
            Self::ClaimsMed => "DOM-05",
            Self::RaQaEsg => "DOM-06",
            Self::SafetyHr => "DOM-07",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id.trim())
    }

    pub fn required_fields(self) -> &'static [&'static str] {
        match self {
            Self::ConstructionSettle => &["contract_sum", "progress_pct", "claimed_amount"],
            Self::RndAudit => &["grant_id", "invoice_total", "card_ledger_total"],
            Self::InsolvencyLegal => &["debtor_id", "principal_krw", "creditor_count"],
            Self::Finance3Way => &["po_no", "gr_qty", "invoice_qty"],
            Self::ClaimsMed => &["claim_no", "policy_limit", "assessed_amount"],
            Self::RaQaEsg => &["batch_no", "cbam_embedded_co2", "coa_passed"],
            Self::SafetyHr => &["site_id", "permit_signed", "work_hours_week"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackCheckInput {
    pub pack: DocsDomainPack,
    pub claimed_total: i64,
    pub evidence_total: i64,
    pub fields_present: Vec<String>,
    pub ocr_predicted: String,
    pub ocr_reference: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackCheckResult {
    pub pack: DocsDomainPack,
    pub mode: EngineMode,
    pub missing_fields: Vec<String>,
    pub variance: i64,
    pub eligible: bool,
    pub char_cer: f64,
}

/// Evaluates a docs pack deterministically without requiring any neural model,
/// and computes character CER via `premium_discount_eval::ranking::cer_char_level`.
pub fn evaluate_pack(input: &PackCheckInput, mode: EngineMode) -> PackCheckResult {
    let missing_fields: Vec<String> = input
        .pack
        .required_fields()
        .iter()
        .filter(|req| !input.fields_present.iter().any(|f| f == *req))
        .map(|s| (*s).to_string())
        .collect();
    let variance = input.claimed_total.saturating_sub(input.evidence_total);
    let eligible = missing_fields.is_empty() && variance <= 0 && input.claimed_total > 0;
    let char_cer = if input.ocr_reference.is_empty() {
        0.0
    } else {
        premium_discount_eval::ranking::cer_char_level(&input.ocr_reference, &input.ocr_predicted)
    };
    PackCheckResult {
        pack: input.pack,
        mode,
        missing_fields,
        variance,
        eligible,
        char_cer,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainMode {
    #[default]
    PowerPlant,
    AiDataCenter,
}

impl DomainMode {
    pub const ALL: [Self; 2] = [Self::PowerPlant, Self::AiDataCenter];

    pub fn key(self) -> &'static str {
        match self {
            Self::PowerPlant => "power_plant",
            Self::AiDataCenter => "aidc",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key.trim() {
            "power_plant" | "plant" | "powerplant" => Some(Self::PowerPlant),
            "aidc" | "ai_data_center" | "datacenter" => Some(Self::AiDataCenter),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Terms {
    pub code: String,
    pub procedure: String,
    pub telemetry: String,
    pub control_room: String,
    pub drawing: String,
    pub site: String,
    pub supervisor: String,
    pub permit: String,
}

impl Default for Terms {
    fn default() -> Self {
        Self {
            code: "KKS".into(),
            procedure: "AOP".into(),
            telemetry: "DCS".into(),
            control_room: "주제어실".into(),
            drawing: "P&ID".into(),
            site: "발전소".into(),
            supervisor: "교대감독".into(),
            permit: "PTW · LOTO".into(),
        }
    }
}

impl Terms {
    pub fn aidc() -> Self {
        Self {
            code: "자산코드".into(),
            procedure: "EOP".into(),
            telemetry: "BMC·DCIM".into(),
            control_room: "NOC 관제실".into(),
            drawing: "MEP·SLD".into(),
            site: "AI 데이터센터".into(),
            supervisor: "교대감독".into(),
            permit: "MOP · LoTo".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeScheme {
    #[default]
    Kks,
    Dcim,
    Iso6346Container,
}

impl CodeScheme {
    #[inline]
    pub fn mask(self) -> &'static [u8] {
        match self {
            Self::Kks => b"99AAA99AA999",
            Self::Dcim => b"AA9AAA99AA99",
            Self::Iso6346Container => b"AAAA9999999",
        }
    }

    pub fn shape(self, code: &str) -> bool {
        let b = code.as_bytes();
        let m = self.mask();
        b.len() == m.len()
            && b.iter()
                .zip(m)
                .all(|(c, m)| if *m == b'9' { c.is_ascii_digit() } else { c.is_ascii_uppercase() })
    }
}

pub fn iso6346_check_digit(prefix10: &[u8]) -> Option<u8> {
    if prefix10.len() != 10 {
        return None;
    }
    let mut sum = 0u32;
    for (i, &b) in prefix10.iter().enumerate() {
        let val: u32 = match b.to_ascii_uppercase() {
            b'0'..=b'9' => (b - b'0') as u32,
            b'A'..=b'Z' => {
                let idx = (b.to_ascii_uppercase() - b'A') as u32;
                let raw = 10 + idx;
                raw + (raw >= 11) as u32 + (raw + (raw >= 11) as u32 >= 22) as u32 + (raw + 2 >= 33 && idx >= 21) as u32
            }
            _ => return None,
        };
        sum += val * (1u32 << i);
    }
    Some(((sum % 11) % 10) as u8)
}

pub fn verify_iso6346_check_digit(code11: &str) -> bool {
    let bytes = code11.as_bytes();
    if bytes.len() != 11 || !bytes[10].is_ascii_digit() {
        return false;
    }
    iso6346_check_digit(&bytes[..10]).is_some_and(|cd| cd == bytes[10] - b'0')
}

fn as_digit(c: char) -> Option<char> {
    match c {
        '0'..='9' => Some(c),
        'O' | 'o' | 'D' | 'Q' => Some('0'),
        'I' | 'l' | 'i' | '|' | 'L' | 'J' | '!' => Some('1'),
        'Z' | 'z' => Some('2'),
        'S' | 's' => Some('5'),
        'G' | 'b' => Some('6'),
        'T' => Some('7'),
        'B' => Some('8'),
        'g' | 'q' => Some('9'),
        _ => None,
    }
}

fn as_letter(c: char) -> Option<char> {
    match c {
        'A'..='Z' => Some(c),
        'a'..='z' => Some(c.to_ascii_uppercase()),
        '0' => Some('O'),
        '1' => Some('I'),
        '2' => Some('Z'),
        '5' => Some('S'),
        '6' => Some('G'),
        '8' => Some('B'),
        _ => None,
    }
}

pub fn repair_code(raw: &str, scheme: CodeScheme) -> String {
    let squeezed: Vec<char> = raw
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '-' | '.' | '_' | ':' | '·'))
        .collect();
    let mask = scheme.mask();
    if squeezed.len() != mask.len() {
        return squeezed.iter().map(|c| c.to_ascii_uppercase()).collect();
    }
    let mut out: String = squeezed
        .iter()
        .zip(mask)
        .map(|(&c, m)| {
            let fixed = if *m == b'9' { as_digit(c) } else { as_letter(c) };
            fixed.unwrap_or(c.to_ascii_uppercase())
        })
        .collect();
    if scheme == CodeScheme::Iso6346Container && out.as_bytes().get(10) == Some(&b'?') {
        if let Some(cd) = iso6346_check_digit(&out.as_bytes()[..10]) {
            out.replace_range(10..11, &cd.to_string());
        }
    }
    out
}

pub fn score_pack_regression(results: &[PackCheckResult]) -> Result<f64> {
    if results.is_empty() {
        anyhow::bail!("no pack results to score");
    }
    let avg_cer = results.iter().map(|r| r.char_cer).sum::<f64>() / results.len() as f64;
    Ok(avg_cer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seven_docs_packs_and_rules_mode_evaluate_deterministically() {
        assert_eq!(DocsDomainPack::ALL.len(), 7);
        for pack in DocsDomainPack::ALL {
            assert_eq!(DocsDomainPack::from_id(pack.id()), Some(pack));
            let res = evaluate_pack(
                &PackCheckInput {
                    pack,
                    claimed_total: 100_000,
                    evidence_total: 120_000,
                    fields_present: pack.required_fields().iter().map(|s| (*s).to_string()).collect(),
                    ocr_predicted: "200,000원".into(),
                    ocr_reference: "200,000원".into(),
                },
                EngineMode::from_str_flag(Some("rules")),
            );
            assert!(res.eligible);
            assert_eq!(res.char_cer, 0.0);
        }
        assert_eq!(repair_code("DC1CDUI0AA02", CodeScheme::Dcim), "DC1CDU10AA02");
        assert!(CodeScheme::Dcim.shape("DC1CDU10AA02"));
    }
}
