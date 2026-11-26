//! 60fps XR live engine loop (`FRAME_BUDGET_US = 16_666`), `ResidencyMode::Pinned` zero-eviction
//! memory pinning, and multi-view `lv_spatial::egoexo::fuse` residual estimation.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use candle_core::Device;
use lv_core::residency::{
    Action, Group, Loaded, ModelId, Phase, Residency, ResidencyMode, SharedResidency,
};
use lv_router::residency_router::{CacheAccessMode, ResidencyRouter};
use lv_spatial::egoexo::{fuse, ViewObs};
use lv_spatial::fit3::{M3, V3};
use serde::{Deserialize, Serialize};

use crate::sandbox_stereo::{SandboxFrameOut, StereoSandboxLive};
use lv_spatial::frame::{DepthMap, Rgb};

pub const TARGET_FPS: u32 = 60;
pub const FRAME_BUDGET_US: u64 = 16_666;
pub const MAX_FUSION_RESIDUAL_M: f64 = 0.02;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MultiViewFusionResult {
    pub fused_xyz: V3,
    pub cov: M3,
    pub residual_m: f64,
    pub within_20mm: bool,
}

/// Fuses multi-camera `ViewObs` via anisotropic Fisher information and returns the RMS residual `(m)`.
pub fn fuse_multi_view_tracks(obs: &[ViewObs]) -> Option<MultiViewFusionResult> {
    if obs.is_empty() {
        return None;
    }
    let (fused_xyz, cov) = fuse(obs)?;
    let sq_sum: f64 = obs
        .iter()
        .map(|o| {
            let dx = o.point[0] - fused_xyz[0];
            let dy = o.point[1] - fused_xyz[1];
            let dz = o.point[2] - fused_xyz[2];
            dx * dx + dy * dy + dz * dz
        })
        .sum();
    let residual_m = (sq_sum / obs.len() as f64).sqrt();
    Some(MultiViewFusionResult {
        fused_xyz,
        cov,
        residual_m,
        within_20mm: residual_m < MAX_FUSION_RESIDUAL_M,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameBudgetReport {
    pub elapsed_us: u64,
    pub budget_us: u64,
    pub within_60fps_budget: bool,
}

pub fn measure_frame_budget(elapsed: Duration) -> FrameBudgetReport {
    let elapsed_us = elapsed.as_micros() as u64;
    FrameBudgetReport {
        elapsed_us,
        budget_us: FRAME_BUDGET_US,
        within_60fps_budget: elapsed_us <= FRAME_BUDGET_US,
    }
}

pub struct SandboxLiveEngine {
    shared_residency: SharedResidency,
    router: ResidencyRouter,
    pub sandbox: StereoSandboxLive,
    pub ticks_executed: u64,
    pub over_budget_ticks: u64,
}

impl SandboxLiveEngine {
    pub fn new(vram_budget_bytes: u64, sandbox: StereoSandboxLive) -> Result<Self> {
        let shared_residency = Residency::shared(Device::Cpu, vram_budget_bytes, ResidencyMode::Pinned);
        {
            let mut guard = shared_residency.lock().expect("residency mutex poisoned");
            guard.pin(ModelId::Siglip2, Arc::new(Loaded::empty(Group::Vision, 512)));
        }
        let router = ResidencyRouter::new(Arc::clone(&shared_residency));
        Ok(Self {
            shared_residency,
            router,
            sandbox,
            ticks_executed: 0,
            over_budget_ticks: 0,
        })
    }

    /// Acquires pinned vision residency via `ReadOnly` (zero eviction / zero lazy reload) and steps stereo keying.
    pub fn tick_stereo_pair(&mut self, left: &Rgb, right: &Rgb) -> Result<(SandboxFrameOut, FrameBudgetReport)> {
        let t0 = Instant::now();
        self.acquire_pinned_vision()?;
        let out = self.sandbox.process_stereo_pair(left, right)?;
        let budget = self.record_budget(t0.elapsed());
        Ok((out, budget))
    }

    /// Acquires pinned vision residency via `ReadOnly` and steps calibrated metric depth keying.
    pub fn tick_calibrated_depth(&mut self, rgb: &Rgb, depth: &DepthMap) -> Result<(SandboxFrameOut, FrameBudgetReport)> {
        let t0 = Instant::now();
        self.acquire_pinned_vision()?;
        let out = self.sandbox.process_calibrated_depth(rgb, depth)?;
        let budget = self.record_budget(t0.elapsed());
        Ok((out, budget))
    }

    pub fn eviction_count(&self) -> usize {
        let guard = self.shared_residency.lock().expect("residency mutex poisoned");
        guard
            .transitions()
            .iter()
            .filter(|t| t.action == Action::Release)
            .count()
    }

    pub fn coexist_hit_count(&self) -> usize {
        let guard = self.shared_residency.lock().expect("residency mutex poisoned");
        guard
            .transitions()
            .iter()
            .filter(|t| t.action == Action::CoexistHit)
            .count()
    }

    fn acquire_pinned_vision(&self) -> Result<()> {
        let phase = Phase::new("vision_sandbox", [Group::Vision]);
        let _handles = self.router.acquire(
            CacheAccessMode::ReadOnly,
            ModelId::Siglip2,
            &phase,
            None,
            |_| unreachable!("Pinned vision ReadOnly slot must never reload weights"),
        )?;
        Ok(())
    }

    fn record_budget(&mut self, elapsed: Duration) -> FrameBudgetReport {
        self.ticks_executed += 1;
        let report = measure_frame_budget(elapsed);
        if !report.within_60fps_budget {
            self.over_budget_ticks += 1;
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lv_spatial::camera::{Intrinsics, Pose};
    use lv_spatial::egoexo::stereo_sigma_z;
    use lv_spatial::fit3::Similarity;
    use lv_spatial::frame::DepthKind;
    use lv_spatial::stereo::{SgbmCfg, StereoRig};
    use lv_spatial::zkey::{SandboxBackground, ZKeyCfg};

    #[test]
    fn pinned_residency_zero_eviction_and_multiview_fusion_under_2cm() {
        let (w, h) = (32u32, 24u32);
        let bg_d = DepthMap { width: w, height: h, kind: DepthKind::Metric, data: vec![6.0; (w * h) as usize] };
        let refs: Vec<&DepthMap> = (0..30).map(|_| &bg_d).collect();
        let bg = SandboxBackground::capture_median(&refs, None, 0.02).unwrap();
        let sandbox = StereoSandboxLive::new(
            StereoRig { focal_px: 300.0, baseline_m: 0.12, subpixel_px: 0.15 },
            SgbmCfg::default(),
            None,
            bg,
            ZKeyCfg::default(),
            Intrinsics { width: w, height: h, fov_deg: 70.0, pitch_deg: 0.0, roll_deg: 0.0 },
            Pose { x: 0.0, z: 0.0, yaw: 0.0, eye_m: 1.5 },
            Similarity { s: 1.0, r: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], t: [0.0, 0.0, 0.0] },
        );
        let mut engine = SandboxLiveEngine::new(64 * 1024 * 1024, sandbox).unwrap();
        let rgb = Rgb::new(w, h, vec![50u8; (w * h * 3) as usize]);

        for _ in 0..5 {
            let (_out, _budget) = engine.tick_calibrated_depth(&rgb, &bg_d).unwrap();
        }
        assert_eq!(engine.eviction_count(), 0, "Pinned residency must never evict weights");
        assert!(engine.coexist_hit_count() >= 5, "all live ticks must hit pinned weights");

        let sz = stereo_sigma_z(3.0, 300.0, 0.12, 0.15);
        let obs = [
            ViewObs { origin: [-2.0, 1.5, 0.0], point: [0.100, 1.100, 3.005], sigma_z: sz, sigma_theta: 0.002, weight: 1.0 },
            ViewObs { origin: [2.0, 1.5, 0.0], point: [0.106, 1.098, 2.997], sigma_z: sz, sigma_theta: 0.002, weight: 1.0 },
            ViewObs { origin: [0.0, 1.5, 5.5], point: [0.097, 1.103, 3.001], sigma_z: sz, sigma_theta: 0.002, weight: 1.0 },
        ];
        let fused = fuse_multi_view_tracks(&obs).unwrap();
        assert!(fused.within_20mm, "residual {} >= 0.02 m", fused.residual_m);
        assert!(measure_frame_budget(Duration::from_micros(8_500)).within_60fps_budget);
        assert!(!measure_frame_budget(Duration::from_micros(20_000)).within_60fps_budget);
    }
}
