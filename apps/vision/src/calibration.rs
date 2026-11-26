//! Sandbox alignment & empty-sandbox depth median capture:
//! SE(3) rigid alignment (`s = 1.0`, zero scale drift) for stereo rigs, similarity fallback for
//! uncalibrated monocular auxiliary cameras, and 30+ frame empty-sandbox background median capture.

use anyhow::{ensure, Context, Result};
use lv_spatial::fit3::{align_rigid, align_similarity, Similarity, V3};
use lv_spatial::frame::{DepthMap, Rgb};
use lv_spatial::zkey::SandboxBackground;

/// Minimum empty-sandbox calibration frame count required before unlocking live Z-Keying.
pub const MIN_EMPTY_SANDBOX_FRAMES: usize = 30;

#[derive(Clone, Debug)]
pub struct SandboxCalibration {
    pub rig_to_sandbox: Similarity,
    pub bg: SandboxBackground,
    pub rmse_m: f64,
}

fn transform_rmse_m(sim: &Similarity, src: &[V3], dst: &[V3]) -> f64 {
    let sq: f64 = src
        .iter()
        .zip(dst)
        .map(|(&p, &q)| {
            let mapped = sim.apply(p);
            (mapped[0] - q[0]).powi(2)
                + (mapped[1] - q[1]).powi(2)
                + (mapped[2] - q[2]).powi(2)
        })
        .sum();
    (sq / src.len().max(1) as f64).sqrt()
}

impl SandboxCalibration {
    /// Fits a rigid SE(3) rig-to-sandbox transform (`s = 1.0`) from >= 3 fiducial markers and
    /// captures an empty-sandbox background depth model from >= 30 static frames.
    pub fn calibrate_rigid(
        rig_markers: &[V3],
        sandbox_markers: &[V3],
        empty_sandbox_frames: &[&DepthMap],
        bg_rgb: Option<Rgb>,
        min_sigma_m: f32,
    ) -> Result<Self> {
        ensure!(
            rig_markers.len() >= 3 && rig_markers.len() == sandbox_markers.len(),
            "rigid sandbox calibration requires >= 3 paired 3D markers (got {} vs {})",
            rig_markers.len(),
            sandbox_markers.len()
        );
        ensure!(
            empty_sandbox_frames.len() >= MIN_EMPTY_SANDBOX_FRAMES,
            "empty-sandbox capture requires >= {MIN_EMPTY_SANDBOX_FRAMES} frames, got {}",
            empty_sandbox_frames.len()
        );

        let rig_to_sandbox = align_rigid(rig_markers, sandbox_markers)
            .context("degenerate marker geometry in align_rigid")?;
        ensure!(
            (rig_to_sandbox.s - 1.0).abs() <= 1e-9,
            "align_rigid must preserve metric scale s=1.0, got {}",
            rig_to_sandbox.s
        );
        let rmse_m = transform_rmse_m(&rig_to_sandbox, rig_markers, sandbox_markers);
        let bg = SandboxBackground::capture_median(empty_sandbox_frames, bg_rgb, min_sigma_m)?;

        Ok(Self { rig_to_sandbox, bg, rmse_m })
    }

    /// Transforms a 3D stereo-rig coordinate `(m)` into canonical sandbox world meters.
    pub fn to_sandbox_xyz(&self, rig_xyz: [f32; 3]) -> [f32; 3] {
        let p = self.rig_to_sandbox.apply([
            rig_xyz[0] as f64,
            rig_xyz[1] as f64,
            rig_xyz[2] as f64,
        ]);
        [p[0] as f32, p[1] as f32, p[2] as f32]
    }
}

/// Computes Sim(3) calibration for an uncalibrated monocular auxiliary camera (`align_similarity`).
pub fn calibrate_monocular_aux_similarity(
    mono_markers: &[V3],
    sandbox_markers: &[V3],
) -> Result<(Similarity, f64)> {
    ensure!(
        mono_markers.len() >= 3 && mono_markers.len() == sandbox_markers.len(),
        "monocular similarity calibration requires >= 3 paired markers"
    );
    let sim = align_similarity(mono_markers, sandbox_markers)
        .context("degenerate marker geometry in align_similarity")?;
    let rmse_m = transform_rmse_m(&sim, mono_markers, sandbox_markers);
    Ok((sim, rmse_m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lv_spatial::frame::DepthKind;

    fn sample_depth_frames(n: usize, w: u32, h: u32, z: f32) -> Vec<DepthMap> {
        (0..n)
            .map(|k| DepthMap {
                width: w,
                height: h,
                kind: DepthKind::Metric,
                data: vec![z + ((k % 3) as f32 - 1.0) * 0.015; (w * h) as usize],
            })
            .collect()
    }

    #[test]
    fn rigid_se3_preserves_metric_scale_and_captures_thirty_empty_frames() {
        let src: Vec<V3> = vec![
            [0.0, 0.0, 2.0],
            [1.5, 0.0, 2.0],
            [0.0, 1.2, 2.0],
            [1.5, 1.2, 3.5],
        ];
        // 90-degree yaw rotation + translation in sandbox world meters (exact SE(3)).
        let dst: Vec<V3> = src
            .iter()
            .map(|p| [-p[2] + 4.0, p[1] + 0.3, p[0] - 1.2])
            .collect();

        let frames = sample_depth_frames(30, 32, 24, 6.0);
        let refs: Vec<&DepthMap> = frames.iter().collect();
        let cal = SandboxCalibration::calibrate_rigid(&src, &dst, &refs, None, 0.02).unwrap();
        assert!((cal.rig_to_sandbox.s - 1.0).abs() < 1e-9);
        assert!(cal.rmse_m < 1e-6, "unexpected rigid RMSE {}", cal.rmse_m);
        assert_eq!(cal.bg.frame_count, 30);

        let mapped = cal.to_sandbox_xyz([1.5, 1.2, 3.5]);
        assert!((mapped[0] - 0.5).abs() < 1e-5);
        assert!((mapped[1] - 1.5).abs() < 1e-5);
        assert!((mapped[2] - 0.3).abs() < 1e-5);

        // Fewer than 30 static frames must be refused.
        assert!(SandboxCalibration::calibrate_rigid(&src, &dst, &refs[..29], None, 0.02).is_err());

        // Auxiliary monocular similarity recovers scale cleanly.
        let mono_src: Vec<V3> = src.iter().map(|p| [p[0] * 0.5, p[1] * 0.5, p[2] * 0.5]).collect();
        let (sim, s_rmse) = calibrate_monocular_aux_similarity(&mono_src, &dst).unwrap();
        assert!((sim.s - 2.0).abs() < 1e-6);
        assert!(s_rmse < 1e-6);
    }
}
