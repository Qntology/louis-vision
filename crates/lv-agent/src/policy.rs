//! Unyielding automation policy:
//! - Default is `OperatingMode::Assist`.
//! - `Adverse` & `Procedural` dispositions NEVER auto-finalize (even at confidence `1.0`).
//! - `Favorable` auto-finalization requires explicit `AutoAuthorization`, all gates clear,
//!   `confidence >= 0.95`, and triggers deterministic 10% (`POST_AUDIT_MOD = 10`) post-audit sampling.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const AUTO_CONFIDENCE_FLOOR: f32 = 0.95;
pub const POST_AUDIT_MOD: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatingMode {
    #[default]
    Assist,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispositionNature {
    Favorable,
    Adverse,
    Procedural,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoAuthorization {
    pub approver_id: String,
    pub legal_basis: String,
    pub authorized_at: DateTime<Utc>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("approver_id cannot be empty")]
    MissingApprover,
    #[error("legal_basis cannot be empty")]
    MissingLegalBasis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    PendingOfficer { reason: String },
    AutoFinalize { post_audit: bool },
}

#[derive(Debug, Clone, Default)]
pub struct AutomationPolicy {
    mode: OperatingMode,
    authorization: Option<AutoAuthorization>,
}

impl AutomationPolicy {
    pub fn mode(&self) -> OperatingMode {
        self.mode
    }

    pub fn authorization(&self) -> Option<&AutoAuthorization> {
        self.authorization.as_ref()
    }

    pub fn enable_auto(
        &mut self,
        approver_id: &str,
        legal_basis: &str,
        now: DateTime<Utc>,
    ) -> Result<AutoAuthorization, PolicyError> {
        if approver_id.trim().is_empty() {
            return Err(PolicyError::MissingApprover);
        }
        if legal_basis.trim().is_empty() {
            return Err(PolicyError::MissingLegalBasis);
        }
        let auth = AutoAuthorization {
            approver_id: approver_id.trim().to_string(),
            legal_basis: legal_basis.trim().to_string(),
            authorized_at: now,
        };
        self.authorization = Some(auth.clone());
        self.mode = OperatingMode::Auto;
        Ok(auth)
    }

    pub fn disable_auto(&mut self) {
        self.mode = OperatingMode::Assist;
    }

    pub fn post_audit_sample(case_id: &str) -> bool {
        let digest = Sha256::digest(case_id.as_bytes());
        let v = u64::from_le_bytes(digest[..8].try_into().unwrap_or([0; 8]));
        v % POST_AUDIT_MOD == 0
    }

    pub fn can_auto_finalize(
        &self,
        nature: DispositionNature,
        confidence: Option<f32>,
        all_gates_clear: bool,
    ) -> bool {
        if self.mode != OperatingMode::Auto || self.authorization.is_none() {
            return false;
        }
        if nature != DispositionNature::Favorable || !all_gates_clear {
            return false;
        }
        confidence.is_some_and(|c| c.is_finite() && c >= AUTO_CONFIDENCE_FLOOR)
    }

    pub fn decide_disposition(
        &self,
        nature: DispositionNature,
        confidence: Option<f32>,
        all_gates_clear: bool,
        case_id: &str,
    ) -> Disposition {
        if self.mode != OperatingMode::Auto || self.authorization.is_none() {
            return Disposition::PendingOfficer { reason: "assist_mode".into() };
        }
        match nature {
            DispositionNature::Adverse => Disposition::PendingOfficer {
                reason: "adverse_never_auto".into(),
            },
            DispositionNature::Procedural => Disposition::PendingOfficer {
                reason: "procedural_requires_officer".into(),
            },
            DispositionNature::Favorable => {
                if !all_gates_clear {
                    Disposition::PendingOfficer { reason: "gates_incomplete".into() }
                } else if !confidence.is_some_and(|c| c.is_finite() && c >= AUTO_CONFIDENCE_FLOOR) {
                    Disposition::PendingOfficer { reason: "confidence_below_0_95".into() }
                } else {
                    Disposition::AutoFinalize {
                        post_audit: Self::post_audit_sample(case_id),
                    }
                }
            }
        }
    }
}
