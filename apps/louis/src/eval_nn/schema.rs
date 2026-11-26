//! Declarative N:N schema, 6 input axes, hard gates, and deterministic seed derivation (`eval_nn`).

use std::collections::BTreeMap;

use lv_agent::gates::{AgentVerdict, OutputForm};
use lv_core::residency::QwenTier;
use lv_doc::writer::guard::N2sfGrade;
use lv_domain::EngineMode;
use serde::{Deserialize, Serialize};

pub const DOCS_NN_HOLDOUT_SEED: u64 = 2026_1126;
pub const DOCS_NN_DIAG_SEED: u64 = 2026_0501;
pub const MASTER_DIAG_SEED: u64 = 2026_0501;
pub const MASTER_HOLDOUT_SEED: u64 = 2026_1126;
pub const FIXED_EVAL_NOW_MS: i64 = lv_sds::seed_tree::FIXED_EVAL_NOW_MS;

pub fn derive_stream_seed(
    split: EvalSplit,
    domain_or_track: &str,
    bundle_idx: u32,
    tag: lv_sds::seed_tree::SeedStreamTag,
) -> u64 {
    let (root, s) = match split {
        EvalSplit::Diag => (MASTER_DIAG_SEED, lv_sds::seed_tree::EvalSplit::Diag),
        EvalSplit::Holdout => (MASTER_HOLDOUT_SEED, lv_sds::seed_tree::EvalSplit::Holdout),
    };
    lv_sds::seed_tree::derive_stream_seed_with_root(root, s, domain_or_track, bundle_idx, tag)
}

pub fn derive_deterministic_wallet_key32(split: EvalSplit, actor_slot: &str) -> [u8; 32] {
    let (root, s) = match split {
        EvalSplit::Diag => (MASTER_DIAG_SEED, lv_sds::seed_tree::EvalSplit::Diag),
        EvalSplit::Holdout => (MASTER_HOLDOUT_SEED, lv_sds::seed_tree::EvalSplit::Holdout),
    };
    lv_sds::seed_tree::derive_deterministic_wallet_key32_with_root(root, s, actor_slot)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalSplit {
    Diag,
    Holdout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuiteSplit {
    DiagCore,
    DiagAll18,
    HoldoutSeal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocsEvalAxis {
    DigitalClean,
    DigitalHostile,
    Scan2DDegraded,
    Field3DPlanar,
    PrivacyContaminated,
    BurstConcurrent,
}

impl DocsEvalAxis {
    pub const ALL: [Self; 6] = [
        Self::DigitalClean,
        Self::DigitalHostile,
        Self::Scan2DDegraded,
        Self::Field3DPlanar,
        Self::PrivacyContaminated,
        Self::BurstConcurrent,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DigitalClean => "digital_clean",
            Self::DigitalHostile => "digital_hostile",
            Self::Scan2DDegraded => "scan_2d_degraded",
            Self::Field3DPlanar => "field_3d_planar",
            Self::PrivacyContaminated => "privacy_contaminated",
            Self::BurstConcurrent => "burst_concurrent",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HardGateCode {
    GateFalseAutoFinalize,
    GateRubberStampBypass,
    GateUnprovenAmountUpward,
    GateOutboundTermOrFactLeak,
    GatePrivacyOrKioskBreach,
    GateInterlockOutOfOrder,
}

pub const fn derive_case_seed(split: EvalSplit, dom_idx: u8, slot_idx: u8) -> u64 {
    let base = match split {
        EvalSplit::Diag => DOCS_NN_DIAG_SEED,
        EvalSplit::Holdout => DOCS_NN_HOLDOUT_SEED,
    };
    let mut z = base
        ^ ((dom_idx as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
        ^ ((slot_idx as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[derive(Debug, Clone, Copy)]
pub struct XorShift64Star {
    state: u64,
}

impl XorShift64Star {
    pub const fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0xDEAD_BEEF_CAFE_BABE
            } else {
                seed
            },
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn range_i64(&mut self, lo: i64, hi_incl: i64) -> i64 {
        let span = (hi_incl - lo + 1).max(1) as u64;
        lo + (self.next_u64() % span) as i64
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CascadeExpectation {
    #[serde(default)]
    pub expect_zero_llm_calls: bool,
    #[serde(default)]
    pub primary_max_tier: Option<QwenTier>,
    #[serde(default)]
    pub expect_selective_4b_escalation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpectedGateSpec {
    pub verdict: AgentVerdict,
    pub allow_auto_finalize: bool,
    pub expected_form: OutputForm,
    pub form_must_override: bool,
    pub expect_fact_fallback: bool,
    pub expect_writer_block: bool,
    pub expect_reposition_first: bool,
    #[serde(default)]
    pub expect_ambiguous_abstain: bool,
    #[serde(default)]
    pub target_n2sf_grade: Option<N2sfGrade>,
    #[serde(default)]
    pub cascade: CascadeExpectation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthCaseSpec {
    pub case_id: String,
    pub split: EvalSplit,
    pub pack_id: String,
    pub axis: DocsEvalAxis,
    pub seed: u64,
    pub source_n2sf: N2sfGrade,
    pub target_n2sf: N2sfGrade,
    pub requested_form: OutputForm,
    pub supplement_count: u32,
    pub injected_faults: Vec<String>,
    pub expected_eligible: bool,
    pub expected_verdict: AgentVerdict,
    pub expected_final_form: OutputForm,
    pub expected_form_overridden: bool,
    pub expected_auto_finalize: bool,
    pub expected_emitted_bytes_zero: bool,
    pub max_allowed_cer: f64,
    pub target_hard_gates_probed: Vec<HardGateCode>,
    pub domain_params: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalCellResult {
    pub case_id: String,
    pub pack_id: String,
    pub axis: DocsEvalAxis,
    pub passed: bool,
    pub cer: f64,
    pub emitted_bytes: usize,
    pub hard_gate_violations: Vec<HardGateCode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocsNnSuiteReport {
    pub split: SuiteSplit,
    pub mode: EngineMode,
    pub defs_rev: String,
    pub total_cells: usize,
    pub passed_cells: usize,
    pub d1_d12_probes_passed: usize,
    pub d1_d12_probes_total: usize,
    pub hard_gate_violations: usize,
    pub max_cer: f64,
    pub dsli_7: f32,
    pub dsli_18: f32,
    pub exit_code: i32,
    pub cells: Vec<EvalCellResult>,
}
