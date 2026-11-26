//! Unified 18-domain (`DOM-01..DOM-18`) forms, 12 `ExtendedFieldFormat` slots, 8 query archetypes (`Q-A1..Q-A8`),
//! zero-digit government ID verification, and 5-Gate legal/physics oracles.

pub mod dom01_construction_settle;
pub mod dom02_rnd_audit;
pub mod dom03_insolvency_legal;
pub mod dom04_finance_3way;
pub mod dom05_claims_med;
pub mod dom06_ra_qa_esg;
pub mod dom07_safety_hr;
pub mod dom08_trade_customs_lc;
pub mod dom09_warehouse_hazmat;
pub mod dom10_retail_srm_settle;
pub mod dom11_public_g2b_cost;
pub mod dom12_fin_kyc_loan;
pub mod dom13_tax_evid_audit;
pub mod dom14_ip_patent_sec;
pub mod dom15_eng_ecn_bom;
pub mod dom16_defense_mro;
pub mod dom17_blind_hr_recruit;
pub mod dom18_audit_internal_gov;

use serde::{Deserialize, Serialize};

pub const SUPPORTED_15_LANGS: [&str; 15] = [
    "ko", "en", "ja", "zh-cn", "zh-tw", "es", "fr", "de", "pt", "it", "nl", "cs", "ar", "ru", "th",
];

pub const ZERO_DIGIT_GOV_ID_TOKENS: [&str; 3] =
    ["[RRN_REDACTED]", "[MYNUMBER_REDACTED]", "[AADHAAR_REDACTED]"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumKind {
    Kks,
    Dcim,
    Iso6346,
    HsCode10,
    BizRegNoKr10,
    Etax24,
    Nsn13,
    Wallet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtendedFieldFormat {
    Identifier,
    TrackingCode,
    ChecksumCode(ChecksumKind),
    Numeric { allow_currency: bool },
    RatioOrPct { to_bps: bool },
    Date,
    TimecodeOrSpan,
    Enum,
    Address,
    Text,
    LineTableSpec,
    AudioTranscriptSpan,
}

impl ExtendedFieldFormat {
    pub const COUNT: usize = 12;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum QueryArchetype {
    QA1,
    QA2,
    QA3,
    QA4,
    QA5,
    QA6,
    QA7,
    QA8,
}

impl QueryArchetype {
    pub const ALL: [Self; 8] = [
        Self::QA1,
        Self::QA2,
        Self::QA3,
        Self::QA4,
        Self::QA5,
        Self::QA6,
        Self::QA7,
        Self::QA8,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::QA1 => "Q-A1",
            Self::QA2 => "Q-A2",
            Self::QA3 => "Q-A3",
            Self::QA4 => "Q-A4",
            Self::QA5 => "Q-A5",
            Self::QA6 => "Q-A6",
            Self::QA7 => "Q-A7",
            Self::QA8 => "Q-A8",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainPackCatalog {
    pub domain_code: &'static str,
    pub pack_id: &'static str,
    pub form_codes: &'static [&'static str],
}

pub fn all_18_domain_catalog() -> Vec<DomainPackCatalog> {
    vec![
        DomainPackCatalog {
            domain_code: "DOM-01",
            pack_id: "construction_settle",
            form_codes: &dom01_construction_settle::DOM01_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-02",
            pack_id: "rnd_audit",
            form_codes: &dom02_rnd_audit::DOM02_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-03",
            pack_id: "insolvency_legal",
            form_codes: &dom03_insolvency_legal::DOM03_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-04",
            pack_id: "finance_3way",
            form_codes: &dom04_finance_3way::DOM04_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-05",
            pack_id: "claims_med",
            form_codes: &dom05_claims_med::DOM05_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-06",
            pack_id: "ra_qa_esg",
            form_codes: &dom06_ra_qa_esg::DOM06_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-07",
            pack_id: "safety_hr",
            form_codes: &dom07_safety_hr::DOM07_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-08",
            pack_id: "trade_customs_lc",
            form_codes: &dom08_trade_customs_lc::DOM08_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-09",
            pack_id: "warehouse_hazmat_pod",
            form_codes: &dom09_warehouse_hazmat::DOM09_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-10",
            pack_id: "retail_srm_settle",
            form_codes: &dom10_retail_srm_settle::DOM10_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-11",
            pack_id: "public_g2b_contract",
            form_codes: &dom11_public_g2b_cost::DOM11_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-12",
            pack_id: "fin_kyc_loan",
            form_codes: &dom12_fin_kyc_loan::DOM12_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-13",
            pack_id: "tax_evid_audit",
            form_codes: &dom13_tax_evid_audit::DOM13_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-14",
            pack_id: "ip_patent_sec",
            form_codes: &dom14_ip_patent_sec::DOM14_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-15",
            pack_id: "eng_ecn_bom",
            form_codes: &dom15_eng_ecn_bom::DOM15_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-16",
            pack_id: "defense_ram_mro",
            form_codes: &dom16_defense_mro::DOM16_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-17",
            pack_id: "hr_comp_privacy",
            form_codes: &dom17_blind_hr_recruit::DOM17_FORMS,
        },
        DomainPackCatalog {
            domain_code: "DOM-18",
            pack_id: "audit_board_gov",
            form_codes: &dom18_audit_internal_gov::DOM18_FORMS,
        },
    ]
}

/// Detects raw (non-redacted) Korean RRN (`NNNNNN-[1-8]NNNNNN`), labeled 12-digit MyNumber,
/// or 12-digit (`4-4-4`) Aadhaar patterns. Must return `false` for clean placeholder tokens and hex digests.
pub fn contains_forbidden_raw_gov_id_digits(text: &str) -> bool {
    let bytes = text.as_bytes();
    // 1. 13-digit Korean RRN: 6 digits, optional separator ('-' or space), [1..=8], 6 digits
    let mut i = 0usize;
    while i + 13 <= bytes.len() {
        let prev_is_alnum = i > 0 && bytes[i - 1].is_ascii_alphanumeric();
        if !prev_is_alnum && bytes[i..i + 6].iter().all(u8::is_ascii_digit) {
            let mm = (bytes[i + 2] - b'0') * 10 + (bytes[i + 3] - b'0');
            let dd = (bytes[i + 4] - b'0') * 10 + (bytes[i + 5] - b'0');
            if (1..=12).contains(&mm) && (1..=31).contains(&dd) {
                let tail_start = if bytes[i + 6] == b'-' || bytes[i + 6] == b' ' {
                    i + 7
                } else {
                    i + 6
                };
                if tail_start + 7 <= bytes.len()
                    && (b'1'..=b'8').contains(&bytes[tail_start])
                    && bytes[tail_start + 1..tail_start + 7].iter().all(u8::is_ascii_digit)
                {
                    let next_is_alnum = bytes
                        .get(tail_start + 7)
                        .is_some_and(u8::is_ascii_alphanumeric);
                    if !next_is_alnum {
                        return true;
                    }
                }
            }
        }
        i += 1;
    }

    // 2. Spaced/hyphenated 4-4-4 Aadhaar (12 digits) or labeled 12-digit MyNumber/Aadhaar
    let lower = text.to_ascii_lowercase();
    let has_gov_label = lower.contains("mynumber")
        || text.contains("마이넘버")
        || text.contains("マイナンバー")
        || lower.contains("aadhaar")
        || text.contains("아드하르");
    let mut j = 0usize;
    while j + 14 <= bytes.len() {
        let prev_is_alnum = j > 0 && bytes[j - 1].is_ascii_alphanumeric();
        let next_is_alnum = bytes.get(j + 14).is_some_and(u8::is_ascii_alphanumeric);
        if !prev_is_alnum
            && !next_is_alnum
            && bytes[j..j + 4].iter().all(u8::is_ascii_digit)
            && matches!(bytes[j + 4], b' ' | b'-')
            && bytes[j + 5..j + 9].iter().all(u8::is_ascii_digit)
            && matches!(bytes[j + 9], b' ' | b'-')
            && bytes[j + 10..j + 14].iter().all(u8::is_ascii_digit)
            && (has_gov_label || bytes[j] >= b'2')
        {
            return true;
        }
        j += 1;
    }
    if has_gov_label {
        let mut run = 0usize;
        for &b in bytes {
            if b.is_ascii_digit() {
                run += 1;
            } else {
                if run == 12 {
                    return true;
                }
                run = 0;
            }
        }
        if run == 12 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_digit_gov_id_placeholders_clean_and_raw_digits_caught() {
        for tok in ZERO_DIGIT_GOV_ID_TOKENS {
            assert!(!contains_forbidden_raw_gov_id_digits(tok));
        }
        assert!(!contains_forbidden_raw_gov_id_digits(
            "근로자 신분: [RRN_REDACTED], [MYNUMBER_REDACTED], [AADHAAR_REDACTED] / 사업자 110-81-41272"
        ));
        assert!(contains_forbidden_raw_gov_id_digits("생년월일 900101-1234567"));
        assert!(contains_forbidden_raw_gov_id_digits("Aadhaar: 2345 6789 0123"));
        assert!(contains_forbidden_raw_gov_id_digits("MyNumber 123456789012"));
        let cat = all_18_domain_catalog();
        assert_eq!(cat.len(), 18);
        let ids: Vec<&str> = cat.iter().map(|c| c.pack_id).collect();
        assert_eq!(ids.as_slice(), &lv_domain::registry::ALL_18_PACK_IDS);
        assert_eq!(SUPPORTED_15_LANGS.len(), 15);
    }
}
