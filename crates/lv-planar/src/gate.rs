//! Active 3D readability gate blocking illegible VLM calls (`ApproachForRead` with 0 VLM calls).

use lv_spatial::camera::{Intrinsics, Pose};
use lv_spatial::fit3::V3;
use lv_spatial::planar::{PhysicalFormFactor, PlanarPatch3D, SurfaceClassHint};
use premium_discount_eval::gate_ab::{compare_gate_arms, ArmRunSummary, GateAbVerdict};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadabilityGateCfg {
    pub min_doc_px_per_mm: f32,
    pub min_plaque_px_per_mm: f32,
    pub min_screen_px_per_mm: f32,
    pub min_view_cos: f32,
    pub max_hole_frac: f32,
    pub min_standoff_m: f32,
    pub edge_margin_px: f32,
    pub glare_avoid_deg: f32,
}

impl Default for ReadabilityGateCfg {
    fn default() -> Self {
        Self {
            min_doc_px_per_mm: 4.0,
            min_plaque_px_per_mm: 5.5,
            min_screen_px_per_mm: 2.5,
            min_view_cos: 0.4226,
            max_hole_frac: 0.45,
            min_standoff_m: 0.22,
            edge_margin_px: 2.0,
            glare_avoid_deg: 10.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadabilityBlockReason {
    ClippedByFrameEdge,
    SpecularGlareHoles,
    SteepViewingAngle,
    LowEffectiveDpi,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApproachForRead {
    pub target_eye_w: V3,
    pub delta_move_w: V3,
    pub target_yaw_rad: f32,
    pub target_pitch_deg: f32,
    pub required_px_per_mm: f32,
    pub observed_px_per_mm: f32,
    pub reason: ReadabilityBlockReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ReadabilityVerdict {
    Pass { min_px_per_mm: f32 },
    NeedReposition(ApproachForRead),
}

#[inline]
pub fn required_density_px_per_mm(patch: &PlanarPatch3D, cfg: &ReadabilityGateCfg) -> f32 {
    if matches!(patch.hint, SurfaceClassHint::DisplayScreen)
        || matches!(patch.form_factor, PhysicalFormFactor::Display16x9)
    {
        cfg.min_screen_px_per_mm
    } else if matches!(patch.hint, SurfaceClassHint::IndustrialNameplate)
        || matches!(patch.form_factor, PhysicalFormFactor::IndustrialPlaque)
    {
        cfg.min_plaque_px_per_mm
    } else {
        cfg.min_doc_px_per_mm
    }
}

pub fn evaluate_readability(
    patch: &PlanarPatch3D,
    intr: &Intrinsics,
    pose: &Pose,
    cfg: &ReadabilityGateCfg,
) -> ReadabilityVerdict {
    let req_dpi = required_density_px_per_mm(patch, cfg);
    let obs_dpi = patch.effective_px_per_mm_u.min(patch.effective_px_per_mm_v);
    let w_lim = intr.width.max(1) as f32 - cfg.edge_margin_px;
    let h_lim = intr.height.max(1) as f32 - cfg.edge_margin_px;

    let clipped = patch.corners_px.iter().any(|&[px, py]| {
        !px.is_finite()
            || !py.is_finite()
            || px < cfg.edge_margin_px
            || px > w_lim
            || py < cfg.edge_margin_px
            || py > h_lim
    });

    let block_reason = if clipped {
        Some(ReadabilityBlockReason::ClippedByFrameEdge)
    } else if patch.depth_hole_frac > cfg.max_hole_frac {
        Some(ReadabilityBlockReason::SpecularGlareHoles)
    } else if patch.view_cos < cfg.min_view_cos {
        Some(ReadabilityBlockReason::SteepViewingAngle)
    } else if obs_dpi < req_dpi {
        Some(ReadabilityBlockReason::LowEffectiveDpi)
    } else {
        None
    };

    let Some(reason) = block_reason else {
        return ReadabilityVerdict::Pass {
            min_px_per_mm: obs_dpi,
        };
    };

    let f_px = intr.focal_px();
    let z_dpi = (0.85 * f_px / (1000.0 * req_dpi.max(0.5) as f64)).max(cfg.min_standoff_m as f64);
    let fill = 0.78f64;
    let pad_w_m = (patch.width_mm as f64 + 12.0) * 1e-3;
    let pad_h_m = (patch.height_mm as f64 + 12.0) * 1e-3;
    let z_fov = (pad_w_m * f_px / (fill * intr.width.max(1) as f64))
        .max(pad_h_m * f_px / (fill * intr.height.max(1) as f64));
    let standoff_m = if z_fov <= z_dpi {
        z_dpi
    } else {
        z_fov.max(cfg.min_standoff_m as f64)
    };

    let dir_w = if reason == ReadabilityBlockReason::SpecularGlareHoles {
        let (s, c) = (cfg.glare_avoid_deg as f64).to_radians().sin_cos();
        [
            c * patch.normal_w[0] + s * patch.tangent_u_w[0],
            c * patch.normal_w[1] + s * patch.tangent_u_w[1],
            c * patch.normal_w[2] + s * patch.tangent_u_w[2],
        ]
    } else {
        patch.normal_w
    };

    let target_eye_w: V3 = [
        patch.center_w[0] + standoff_m * dir_w[0],
        patch.center_w[1] + standoff_m * dir_w[1],
        patch.center_w[2] + standoff_m * dir_w[2],
    ];
    let cur_eye_w: V3 = [pose.x as f64, pose.eye_m as f64, pose.z as f64];
    let delta_move_w: V3 = [
        target_eye_w[0] - cur_eye_w[0],
        target_eye_w[1] - cur_eye_w[1],
        target_eye_w[2] - cur_eye_w[2],
    ];

    let look = [
        patch.center_w[0] - target_eye_w[0],
        patch.center_w[1] - target_eye_w[1],
        patch.center_w[2] - target_eye_w[2],
    ];
    let horiz = look[0].hypot(look[2]).max(1e-9);
    let target_yaw_rad = look[2].atan2(look[0]) as f32;
    let target_pitch_deg = look[1].atan2(horiz).to_degrees() as f32;

    ReadabilityVerdict::NeedReposition(ApproachForRead {
        target_eye_w,
        delta_move_w,
        target_yaw_rad,
        target_pitch_deg,
        required_px_per_mm: req_dpi,
        observed_px_per_mm: obs_dpi,
        reason,
    })
}

pub fn evaluate_readability_gate_ab(
    gated_on: &ArmRunSummary,
    ungated_off: &ArmRunSummary,
) -> GateAbVerdict {
    compare_gate_arms(gated_on, ungated_off)
}
