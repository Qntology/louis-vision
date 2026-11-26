//! Unified 18-domain pack registry (`DOM-01..DOM-18`), `builtin_18()`, and metadata digest.

use serde::{Deserialize, Serialize};

pub const ALL_18_PACK_IDS: [&str; 18] = [
    "construction_settle",
    "rnd_audit",
    "insolvency_legal",
    "finance_3way",
    "claims_med",
    "ra_qa_esg",
    "safety_hr",
    "trade_customs_lc",
    "warehouse_hazmat_pod",
    "retail_srm_settle",
    "public_g2b_contract",
    "fin_kyc_loan",
    "tax_evid_audit",
    "ip_patent_sec",
    "eng_ecn_bom",
    "defense_ram_mro",
    "hr_comp_privacy",
    "audit_board_gov",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DomainPackMeta {
    pub code: &'static str,
    pub id: &'static str,
    pub label_ko: &'static str,
    pub required_fields: &'static [&'static str],
    pub default_retention_days: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackSignals {
    pub discrepancy_count: usize,
    pub expired: bool,
    pub signature_invalid: bool,
    pub unauthorized_override: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainEvalRequest {
    pub pack_id: String,
    pub claimed_total: i64,
    pub evidence_total: i64,
    pub fields_present: Vec<String>,
    pub ocr_predicted: String,
    pub ocr_reference: String,
    pub signals: PackSignals,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainPackRegistry {
    packs: Vec<DomainPackMeta>,
}

impl DomainPackRegistry {
    /// Returns all 18 enterprise domain packs (`DOM-01..DOM-18`) non-destructively.
    pub fn builtin_18() -> Self {
        Self {
            packs: vec![
                DomainPackMeta { code: "DOM-01", id: "construction_settle", label_ko: "건설·하도급 기성", required_fields: &["contract_sum", "progress_pct", "claimed_amount"], default_retention_days: 1825 },
                DomainPackMeta { code: "DOM-02", id: "rnd_audit", label_ko: "국가R&D 연구비 정산", required_fields: &["grant_id", "invoice_total", "card_ledger_total"], default_retention_days: 1825 },
                DomainPackMeta { code: "DOM-03", id: "insolvency_legal", label_ko: "회생·도산 채권조사", required_fields: &["debtor_id", "principal_krw", "creditor_count"], default_retention_days: 730 },
                DomainPackMeta { code: "DOM-04", id: "finance_3way", label_ko: "기업 매입·3-Way 대사", required_fields: &["po_no", "gr_qty", "invoice_qty"], default_retention_days: 1825 },
                DomainPackMeta { code: "DOM-05", id: "claims_med", label_ko: "보험·의료 실손 심사", required_fields: &["claim_no", "policy_limit", "assessed_amount"], default_retention_days: 730 },
                DomainPackMeta { code: "DOM-06", id: "ra_qa_esg", label_ko: "품질·CBAM·인허가", required_fields: &["batch_no", "cbam_embedded_co2", "coa_passed"], default_retention_days: 1825 },
                DomainPackMeta { code: "DOM-07", id: "safety_hr", label_ko: "현장 LOTO·노무안전", required_fields: &["site_id", "permit_signed", "work_hours_week"], default_retention_days: 1825 },
                DomainPackMeta { code: "DOM-08", id: "trade_customs_lc", label_ko: "무역·신용장·통관", required_fields: &["po_no", "ci_amount", "bl_date", "lc_limit"], default_retention_days: 1825 },
                DomainPackMeta { code: "DOM-09", id: "warehouse_hazmat_pod", label_ko: "물류창고·보세·위험물", required_fields: &["container_id", "bonded_cleared", "un_hazmat"], default_retention_days: 1825 },
                DomainPackMeta { code: "DOM-10", id: "retail_srm_settle", label_ko: "유통·SRM 대금정산", required_fields: &["contract_kind", "promo_ratio", "payout_ymd"], default_retention_days: 1825 },
                DomainPackMeta { code: "DOM-11", id: "public_g2b_contract", label_ko: "공공입찰·원가계산", required_fields: &["rule_kind", "admin_krw", "profit_krw"], default_retention_days: 1825 },
                DomainPackMeta { code: "DOM-12", id: "fin_kyc_loan", label_ko: "여신·KYC·책무구조", required_fields: &["ubo_verified", "accountability_code", "shareholder_sum"], default_retention_days: 730 },
                DomainPackMeta { code: "DOM-13", id: "tax_evid_audit", label_ko: "세무증빙·접대비 감사", required_fields: &["etax_24_id", "transmit_ymd", "vat_credit"], default_retention_days: 1825 },
                DomainPackMeta { code: "DOM-14", id: "ip_patent_sec", label_ko: "특허·국가핵심기술 보안", required_fields: &["n2sf_grade", "motie_approval_no", "clean_cdr"], default_retention_days: 1825 },
                DomainPackMeta { code: "DOM-15", id: "eng_ecn_bom", label_ko: "도면 ECN·BOM 대사", required_fields: &["ecn_no", "title_block_5of5", "bom_qty"], default_retention_days: 1825 },
                DomainPackMeta { code: "DOM-16", id: "defense_ram_mro", label_ko: "방산 MRO·ECP 형상", required_fields: &["plaque_dpi", "ecp_no", "coc_verified"], default_retention_days: 1825 },
                DomainPackMeta { code: "DOM-17", id: "hr_comp_privacy", label_ko: "인사·블라인드 채용", required_fields: &["art4_3_clean", "career_days", "blind_clean"], default_retention_days: 730 },
                DomainPackMeta { code: "DOM-18", id: "audit_board_gov", label_ko: "내부감사·수의계약 통제", required_fields: &["single_quote_krw", "annual_count", "board_cap"], default_retention_days: 1825 },
            ],
        }
    }

    pub fn len(&self) -> usize {
        self.packs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packs.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &DomainPackMeta> {
        self.packs.iter()
    }

    pub fn get(&self, id: &str) -> Option<&DomainPackMeta> {
        self.packs.iter().find(|p| p.id == id || p.code == id)
    }

    pub fn builtin_packs_rev(&self) -> String {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for p in &self.packs {
            for &b in p.code.as_bytes().iter().chain(b":").chain(p.id.as_bytes()) {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        format!("{:08x}", (h ^ (h >> 32)) as u32)
    }
}
