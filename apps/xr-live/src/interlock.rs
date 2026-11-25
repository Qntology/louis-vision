//! Declarative 3-step `StageInterlock`:
//!   1. `MetricZoneClear` — 0 voxels/points inside hazard OBBs AND every active view `valid_ratio >= 0.85`
//!   2. `RigSyncSteady` — multi-view `egoexo` fusion residual `< 0.02 m`
//!   3. `TemporalGateClear` — `lv_sds::temporal::temporal_gate` `!Held`
//! Any violation trips `/louis/interlock/halt`, blocks both `AutoTimecode` and `StageCueRemote`,
//! and appends to a hash-chained `AuditLog`.

use std::path::{Path, PathBuf};

use anyhow::Result;
use lv_sds::temporal::{temporal_gate, GateConfig, TemporalOutcome};
use premium_discount_sds::audit::{verify_file, AuditLog, Level};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::live_loop::MAX_FUSION_RESIDUAL_M;

pub const MIN_STEREO_VALID_RATIO: f32 = 0.85;
pub const INTERLOCK_HALT_OSC: &str = "/louis/interlock/halt";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StageHazardObb {
    pub name: String,
    pub min_xyz: [f32; 3],
    pub max_xyz: [f32; 3],
}

impl StageHazardObb {
    pub fn contains(&self, p: [f32; 3]) -> bool {
        p[0].is_finite()
            && p[1].is_finite()
            && p[2].is_finite()
            && p[0] >= self.min_xyz[0]
            && p[0] <= self.max_xyz[0]
            && p[1] >= self.min_xyz[1]
            && p[1] <= self.max_xyz[1]
            && p[2] >= self.min_xyz[2]
            && p[2] <= self.max_xyz[2]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CueTriggerSource {
    AutoTimecode,
    StageCueRemote,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InterlockInputs {
    pub active_view_valid_ratios: Vec<f32>,
    pub metric_points_m: Vec<[f32; 3]>,
    pub fusion_residual_m: Option<f64>,
    pub residual_history_m: Vec<f64>,
    pub residual_sds_sd_m: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterlockDecision {
    Clear {
        temporal_outcome: String,
    },
    Halt {
        reasons: Vec<String>,
        osc_address: String,
        blocked_auto_timecode: bool,
        blocked_cue_remote: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InterlockDispatch {
    pub source: CueTriggerSource,
    pub cue_name: String,
    pub allowed: bool,
    pub osc_address: String,
    pub decision: InterlockDecision,
}

pub struct StageInterlock {
    pub hazards: Vec<StageHazardObb>,
    gate_cfg: GateConfig,
    audit: AuditLog,
    tick: u64,
}

impl StageInterlock {
    pub fn new_memory(hazards: Vec<StageHazardObb>, audit_capacity: usize) -> Self {
        Self {
            hazards,
            gate_cfg: GateConfig::default(),
            audit: AuditLog::memory(audit_capacity.max(8)),
            tick: 0,
        }
    }

    pub fn new_persistent(
        hazards: Vec<StageHazardObb>,
        log_dir: &Path,
        session_name: &str,
        audit_capacity: usize,
    ) -> Result<Self> {
        let audit = AuditLog::open(log_dir, session_name, audit_capacity.max(8))?;
        Ok(Self {
            hazards,
            gate_cfg: GateConfig::default(),
            audit,
            tick: 0,
        })
    }

    pub fn audit_path(&self) -> Option<PathBuf> {
        self.audit.path().map(Path::to_path_buf)
    }

    /// Evaluates Steps 1-3 cleanly, logging to the hash-chained `AuditLog` on every check.
    pub fn evaluate(&mut self, inputs: &InterlockInputs) -> InterlockDecision {
        self.tick += 1;
        let mut reasons = Vec::new();

        // Step 1: MetricZoneClear — stereo coverage >= 0.85 across all active views & 0 hazard intrusions.
        if inputs.active_view_valid_ratios.is_empty() {
            reasons.push("metric_zone_clear: no active stereo views".to_string());
        } else {
            for (idx, &ratio) in inputs.active_view_valid_ratios.iter().enumerate() {
                if !ratio.is_finite() || ratio < MIN_STEREO_VALID_RATIO {
                    reasons.push(format!(
                        "metric_zone_clear: view[{idx}] valid_ratio={ratio:.3} < {MIN_STEREO_VALID_RATIO:.2}"
                    ));
                }
            }
        }

        for obb in &self.hazards {
            let count = inputs
                .metric_points_m
                .iter()
                .filter(|&&p| obb.contains(p))
                .count();
            if count > 0 {
                reasons.push(format!(
                    "metric_zone_clear: hazard_obb_occupied:{}:{count}",
                    obb.name
                ));
            }
        }

        // Step 2: RigSyncSteady — multi-view fusion residual < 0.02 m.
        let residual_val = match inputs.fusion_residual_m {
            Some(r) if r.is_finite() && r < MAX_FUSION_RESIDUAL_M => r,
            Some(r) => {
                reasons.push(format!(
                    "rig_sync_steady: fusion_residual_m={r:.4} >= {MAX_FUSION_RESIDUAL_M:.2}"
                ));
                r
            }
            None => {
                reasons.push("rig_sync_steady: fusion_residual_m missing".to_string());
                f64::NAN
            }
        };

        // Step 3: TemporalGateClear — lv-sds temporal_gate must not return Held.
        let verdict = if residual_val.is_finite() {
            temporal_gate(
                &inputs.residual_history_m,
                residual_val,
                inputs.residual_sds_sd_m,
                &[],
                &[],
                &self.gate_cfg,
            )
        } else {
            temporal_gate(
                &inputs.residual_history_m,
                0.0,
                inputs.residual_sds_sd_m,
                &[],
                &[],
                &self.gate_cfg,
            )
        };
        if verdict.outcome == TemporalOutcome::Held || !verdict.passes() {
            reasons.push(format!("temporal_gate_clear: held ({})", verdict.reason));
        }

        if reasons.is_empty() {
            self.audit.log_with_tick(
                "interlock",
                Level::Info,
                "stage_interlock_clear",
                Some(json!({
                    "fusion_residual_m": inputs.fusion_residual_m,
                    "temporal_outcome": format!("{:?}", verdict.outcome),
                })),
                Some(self.tick),
            );
            InterlockDecision::Clear {
                temporal_outcome: format!("{:?}", verdict.outcome),
            }
        } else {
            self.audit.log_with_tick(
                "interlock",
                Level::Info,
                "stage_interlock_halt",
                Some(json!({
                    "osc": INTERLOCK_HALT_OSC,
                    "reasons": reasons,
                })),
                Some(self.tick),
            );
            InterlockDecision::Halt {
                reasons,
                osc_address: INTERLOCK_HALT_OSC.to_string(),
                blocked_auto_timecode: true,
                blocked_cue_remote: true,
            }
        }
    }

    /// Attempts to dispatch a stage cue; any halted interlock blocks both `AutoTimecode` and `StageCueRemote`.
    pub fn dispatch_cue(
        &mut self,
        source: CueTriggerSource,
        cue_name: &str,
        inputs: &InterlockInputs,
    ) -> Result<InterlockDispatch> {
        let decision = self.evaluate(inputs);
        let (allowed, osc_address) = match &decision {
            InterlockDecision::Clear { .. } => {
                let clean = cue_name.trim().trim_start_matches('/');
                (true, format!("/louis/cue/{clean}"))
            }
            InterlockDecision::Halt { osc_address, .. } => (false, osc_address.clone()),
        };

        self.audit.log_with_tick(
            "cue_dispatch",
            Level::Info,
            if allowed { "cue_dispatched" } else { "cue_blocked_by_interlock" },
            Some(json!({
                "source": source,
                "cue": cue_name,
                "allowed": allowed,
                "osc": osc_address,
            })),
            Some(self.tick),
        );

        Ok(InterlockDispatch {
            source,
            cue_name: cue_name.to_string(),
            allowed,
            osc_address,
            decision,
        })
    }
}

/// Verifies a hash-chained `StageInterlock` JSONL audit file and returns verified line count.
pub fn verify_audit_chain(path: &Path) -> Result<usize> {
    let report = verify_file(path).map_err(|e| anyhow::anyhow!("audit chain verification failed: {e:?}"))?;
    Ok(report.lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steady_inputs() -> InterlockInputs {
        InterlockInputs {
            active_view_valid_ratios: vec![0.91, 0.88],
            metric_points_m: vec![[0.0, 1.2, 3.0], [1.5, 1.1, 3.5]],
            fusion_residual_m: Some(0.006),
            residual_history_m: (0..20)
                .map(|i| 0.005 + 0.0005 * (((i * 7 + 3) % 5) as f64 - 2.0))
                .collect(),
            residual_sds_sd_m: Some(0.002),
        }
    }

    #[test]
    fn three_step_interlock_triggers_halt_and_verifies_hash_chain() {
        let dir = std::env::temp_dir().join(format!("louis_stage_interlock_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let hazards = vec![StageHazardObb {
            name: "pyro_pit".into(),
            min_xyz: [-1.0, 0.0, 0.2],
            max_xyz: [1.0, 2.5, 1.0],
        }];
        let mut il = StageInterlock::new_persistent(hazards, &dir, "interlock_test", 128).unwrap();

        // 0. Clear baseline allows both AutoTimecode & StageCueRemote.
        let ok = il.dispatch_cue(CueTriggerSource::AutoTimecode, "c1", &steady_inputs()).unwrap();
        assert!(ok.allowed && ok.osc_address == "/louis/cue/c1");

        // 1a. Low stereo valid ratio (< 0.85) halts both AutoTimecode & StageCueRemote.
        let mut low_cov = steady_inputs();
        low_cov.active_view_valid_ratios[1] = 0.72;
        let h_cov_auto = il.dispatch_cue(CueTriggerSource::AutoTimecode, "c2", &low_cov).unwrap();
        let h_cov_rem = il.dispatch_cue(CueTriggerSource::StageCueRemote, "c2", &low_cov).unwrap();
        assert!(!h_cov_auto.allowed && h_cov_auto.osc_address == INTERLOCK_HALT_OSC);
        assert!(!h_cov_rem.allowed && h_cov_rem.osc_address == INTERLOCK_HALT_OSC);

        // 1b. Point inside hazard OBB trips MetricZoneClear.
        let mut in_obb = steady_inputs();
        in_obb.metric_points_m.push([0.1, 1.0, 0.5]);
        let h_obb = il.dispatch_cue(CueTriggerSource::StageCueRemote, "c3", &in_obb).unwrap();
        assert!(!h_obb.allowed);

        // 2. Rig sync residual >= 0.02 m trips RigSyncSteady.
        let mut drift = steady_inputs();
        drift.fusion_residual_m = Some(0.025);
        drift.residual_history_m = vec![]; // isolate Step 2
        let h_drift = il.dispatch_cue(CueTriggerSource::AutoTimecode, "c4", &drift).unwrap();
        assert!(!h_drift.allowed);

        // 3. Sudden residual spike inside allowed <0.02 m band triggers TemporalGateClear::Held.
        let mut spike = steady_inputs();
        spike.fusion_residual_m = Some(0.018); // < 0.02, but sudden spike over steady ~0.005 baseline
        let h_temp = il.dispatch_cue(CueTriggerSource::StageCueRemote, "c5", &spike).unwrap();
        assert!(!h_temp.allowed);
        if let InterlockDecision::Halt { reasons, .. } = &h_temp.decision {
            assert!(reasons.iter().any(|r| r.contains("temporal_gate_clear")));
        } else {
            panic!("expected TemporalGateClear halt");
        }

        let path = il.audit_path().expect("persistent log path");
        let verified_lines = verify_audit_chain(&path).expect("hash-chained audit log must verify");
        assert!(verified_lines >= 12);
        drop(il);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
