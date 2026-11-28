//! Product-path forensic WORM audit logger (`apps/louis/src/audit.rs`):
//! - 4 statutory retention classes (`1,825d / 1,095d / 730d / 365d`)
//! - 11-field mandatory paradata (`SealedAuditPayload`)
//! - Dual hash-chained JSONL writer backed by Tier 1 `lv_sds::{AuditChain, ReflectionFlushGuard}`
//! - 4-vector tamper verifier (`verify_worm_audit_jsonl`) decoupled from `eval_nn`.

use std::path::Path;

use anyhow::Result;
use lv_agent::gates::AgentVerdict;
use lv_agent::policy::Disposition;
use lv_doc::writer::guard::N2sfGrade;
use lv_domain::EngineMode;
use lv_sds::{AuditChain, AuditEntry, ReflectionFlushGuard, ReflectionSyncPolicy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionClass {
    HighImpactAi5Y,
    PrivacyPermission3Y,
    PrivacyAccessSensitive2Y,
    PrivacyAccessStandard1Y,
}

impl RetentionClass {
    pub const fn min_retention_days(self) -> u32 {
        match self {
            Self::HighImpactAi5Y => 1_825,
            Self::PrivacyPermission3Y => 1_095,
            Self::PrivacyAccessSensitive2Y => 730,
            Self::PrivacyAccessStandard1Y => 365,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SealedAuditPayload {
    pub timestamp_utc: String,
    pub case_id: String,
    pub manifest_rev: String,
    pub docs_defs_rev: String,
    pub engine_mode: EngineMode,
    pub n2sf_grade: N2sfGrade,
    pub verdict: AgentVerdict,
    pub confidence: Option<f32>,
    pub disposition: Disposition,
    pub officer_id: Option<String>,
    pub justification: Option<String>,
    pub retention_class: RetentionClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_hmac_tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TamperFinding {
    SequenceGap { index: usize, expected_seq: u64, actual_seq: u64 },
    BrokenPrevLink { seq: u64, expected_prev: String, actual_prev: String },
    PayloadHashMismatch { seq: u64, computed_hash: String, recorded_hash: String },
    AnchorHeadMismatch { expected_head: String, actual_head: String },
    MissingMandatoryParadata { seq: u64, missing_field: &'static str },
}

pub const REQUIRED_11_PARADATA_KEYS: [&str; 11] = [
    "timestamp_utc",
    "case_id",
    "manifest_rev",
    "docs_defs_rev",
    "engine_mode",
    "n2sf_grade",
    "verdict",
    "confidence",
    "disposition",
    "officer_id",
    "justification",
];

/// Verifies every hash link, sequence counter, 11 mandatory paradata keys, and final head anchor
/// on a WORM audit JSONL stream.
pub fn verify_worm_audit_jsonl(
    jsonl_bytes: &[u8],
    genesis_tag: &str,
    sealed_head_anchor: &str,
) -> Result<std::result::Result<usize, TamperFinding>> {
    let text = std::str::from_utf8(jsonl_bytes)?;
    let mut expected_prev = genesis_tag.to_string();
    let mut expected_seq = 1u64;
    let mut count = 0usize;

    for (idx, raw_line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let entry: AuditEntry = serde_json::from_str(raw_line)?;
        if entry.line.seq != expected_seq {
            return Ok(Err(TamperFinding::SequenceGap {
                index: idx,
                expected_seq,
                actual_seq: entry.line.seq,
            }));
        }
        if entry.prev != expected_prev {
            return Ok(Err(TamperFinding::BrokenPrevLink {
                seq: entry.line.seq,
                expected_prev,
                actual_prev: entry.prev,
            }));
        }
        let body = serde_json::to_string(&entry.line)?;
        let mut h = Sha256::new();
        h.update(entry.prev.as_bytes());
        h.update(b":");
        h.update(body.as_bytes());
        let computed: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if computed != entry.hash {
            return Ok(Err(TamperFinding::PayloadHashMismatch {
                seq: entry.line.seq,
                computed_hash: computed,
                recorded_hash: entry.hash,
            }));
        }
        let Some(data_obj) = entry.line.data.as_ref().and_then(|v| v.as_object()) else {
            return Ok(Err(TamperFinding::MissingMandatoryParadata {
                seq: entry.line.seq,
                missing_field: "data",
            }));
        };
        for &key in &REQUIRED_11_PARADATA_KEYS {
            if !data_obj.contains_key(key) {
                return Ok(Err(TamperFinding::MissingMandatoryParadata {
                    seq: entry.line.seq,
                    missing_field: key,
                }));
            }
        }
        expected_prev = entry.hash;
        expected_seq += 1;
        count += 1;
    }

    if expected_prev != sealed_head_anchor {
        return Ok(Err(TamperFinding::AnchorHeadMismatch {
            expected_head: sealed_head_anchor.to_string(),
            actual_head: expected_prev,
        }));
    }
    Ok(Ok(count))
}

/// Product-path hash-chained WORM logger writing directly to disk via `ReflectionFlushGuard`.
pub struct WormAuditLogger {
    chain: AuditChain,
    guard: Option<ReflectionFlushGuard>,
}

impl Default for WormAuditLogger {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl WormAuditLogger {
    pub fn in_memory() -> Self {
        Self {
            chain: AuditChain::default(),
            guard: None,
        }
    }

    pub fn open_file(path: &Path, sync_policy: ReflectionSyncPolicy) -> Result<Self> {
        let guard = ReflectionFlushGuard::open(path, sync_policy)?;
        Ok(Self {
            chain: AuditChain::default(),
            guard: Some(guard),
        })
    }

    pub fn record(
        &mut self,
        tick: u64,
        cat: &str,
        msg: &str,
        payload: &SealedAuditPayload,
    ) -> Result<AuditEntry> {
        let data = serde_json::to_value(payload)?;
        let entry = self
            .chain
            .append(i64::try_from(tick).unwrap_or(i64::MAX), cat, msg, Some(data))
            .clone();
        if let Some(ref g) = self.guard {
            g.write_entry(&entry)?;
        }
        Ok(entry)
    }

    pub fn head_anchor(&self) -> &str {
        self.chain.head()
    }

    pub fn len(&self) -> usize {
        self.chain.entries().len()
    }

    pub fn is_empty(&self) -> bool {
        self.chain.entries().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lv_sds::audit::GENESIS;

    #[test]
    fn product_worm_audit_logger_seals_and_verifies_disk_jsonl() {
        let dir = std::env::temp_dir().join(format!("louis-worm-audit-{}", std::process::id()));
        let log_path = dir.join("product_audit.jsonl");
        let _ = std::fs::remove_dir_all(&dir);

        let head = {
            let mut logger = WormAuditLogger::open_file(&log_path, ReflectionSyncPolicy::OsFlush).unwrap();
            let payload = SealedAuditPayload {
                timestamp_utc: "2026-11-28T03:00:00Z".into(),
                case_id: "PROD-DOM11-01".into(),
                manifest_rev: "builtin18".into(),
                docs_defs_rev: "prod_v1".into(),
                engine_mode: EngineMode::RulesOnly,
                n2sf_grade: N2sfGrade::Sensitive,
                verdict: AgentVerdict::G3EligibleNormal,
                confidence: Some(0.99),
                disposition: Disposition::PendingOfficer {
                    reason: "high_impact_domain_requires_officer".into(),
                },
                officer_id: Some("OFF-01".into()),
                justification: None,
                retention_class: RetentionClass::HighImpactAi5Y,
                subject_hmac_tag: None,
            };
            logger.record(1, "workflow", "case_sealed", &payload).unwrap();
            logger.head_anchor().to_string()
        };

        let bytes = std::fs::read(&log_path).unwrap();
        assert_eq!(verify_worm_audit_jsonl(&bytes, GENESIS, &head).unwrap(), Ok(1));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
