//! Shared UI N:N scenario definition & FNV-1a revision fingerprint (`ui_defs_rev()`).

use premium_discount_eval::GateExit;
use premium_discount_eval::rule::EvalCell;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

pub const UI_NN_DIAG_SEED: u64 = 2026_0501;
pub const UI_NN_HOLDOUT_SEED: u64 = 2026_1126;

const UI_NN_DEFS_RAW: &str = include_str!("../../../shared-ui/data/ui_nn_scenarios.json");
static UI_DEFS_REV: OnceLock<String> = OnceLock::new();

/// FNV-1a 64->32 XOR-fold 8-hex digest matching `sandbox-core::nn::defs_rev`.
pub fn ui_defs_rev() -> &'static str {
    UI_DEFS_REV.get_or_init(|| {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in UI_NN_DEFS_RAW.as_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{:08x}", (h ^ (h >> 32)) as u32)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UiHardGate {
    GateDuplicateAnchor,
    GateStageVramContention,
    GatePrivacyPixelLeak,
    GateSqlGuardBypass,
    GateWrongFormUnoverridden,
    GateRubberStampOrOutboundLeak,
    GateInterlockOrBlackoutMiss,
    GateKioskOrWanBreach,
}

mod gate_exit_serde {
    use premium_discount_eval::GateExit;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &GateExit, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match v {
            GateExit::Pass => "pass",
            GateExit::Regressed => "regressed",
            GateExit::AuditError => "audit_error",
        })
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<GateExit, D::Error> {
        let raw = String::deserialize(d)?;
        match raw.as_str() {
            "pass" | "Pass" => Ok(GateExit::Pass),
            "regressed" | "Regressed" => Ok(GateExit::Regressed),
            "audit_error" | "AuditError" => Ok(GateExit::AuditError),
            other => Err(serde::de::Error::custom(format!("unknown GateExit: {other}"))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiNnSuiteReport {
    pub ui_defs_rev: String,
    pub evaluated_cells: usize,
    pub cells: Vec<EvalCell>,
    pub hard_gate_hits: Vec<(String, UiHardGate)>,
    pub holdout_passed: Option<(usize, usize)>,
    #[serde(with = "gate_exit_serde")]
    pub exit: GateExit,
}
