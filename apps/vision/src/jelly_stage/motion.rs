//! BPM-synchronized volume-preserving (`det(S) = 1.0`) squash-and-stretch motion generator for `jelly_stage`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadPropKind {
    None,
    Crown,
    Headphones,
    Cap,
    Halo,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BpmMotif {
    GrooveWave,
    BboyFreeze,
    ElasticHop,
    MoveToXz { target_xz: [f32; 2] },
}

/// Smooth-min SDF capsule with guaranteed volume-preserving squash-and-stretch (`sx * sy * sz == 1.0`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JellySdfCapsule {
    pub id: String,
    pub p0: [f32; 3],
    pub p1: [f32; 3],
    pub radius_m: f32,
    pub scale_xyz: [f32; 3],
    pub tube_bulge_k: f32,
    pub smin_blend_m: f32,
}

impl JellySdfCapsule {
    /// Builds a capsule whose scale tensor has exact unit determinant (`det(S) = 1.0`).
    pub fn volume_preserving(
        id: impl Into<String>,
        p0: [f32; 3],
        p1: [f32; 3],
        radius_m: f32,
        stretch_y: f32,
        tube_bulge_k: f32,
        smin_blend_m: f32,
    ) -> Self {
        let sy = stretch_y.clamp(0.65, 1.55);
        let sxz = (1.0 / sy).sqrt();
        Self {
            id: id.into(),
            p0,
            p1,
            radius_m: radius_m.max(0.03),
            scale_xyz: [sxz, sy, sxz],
            tube_bulge_k: tube_bulge_k.clamp(0.0, 0.5),
            smin_blend_m: smin_blend_m.clamp(0.01, 0.25),
        }
    }

    pub fn scale_det(&self) -> f32 {
        self.scale_xyz[0] * self.scale_xyz[1] * self.scale_xyz[2]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JellyPoseFrame {
    pub beat_phase: f32,
    pub root_xyz: [f32; 3],
    pub capsules: Vec<JellySdfCapsule>,
    pub beat_env: f32,
}

/// Evaluates deterministic goal-driven BPM motion at time `t_s` for an actor at `origin_xz`.
pub fn evaluate_bpm_motif(
    motif: BpmMotif,
    origin_xz: [f32; 2],
    bpm: f32,
    t_s: f32,
) -> JellyPoseFrame {
    let bpm_safe = bpm.clamp(40.0, 220.0);
    let beats = t_s.max(0.0) * (bpm_safe / 60.0);
    let phase = (beats * std::f32::consts::TAU).sin();
    let beat_env = (0.5 + 0.5 * (beats * std::f32::consts::TAU).cos()).clamp(0.15, 1.0);

    let (root_xyz, stretch_y, arm_lift) = match motif {
        BpmMotif::GrooveWave => ([origin_xz[0], 0.0, origin_xz[1]], 1.0 + 0.14 * phase, 0.35 * phase),
        BpmMotif::BboyFreeze => ([origin_xz[0], 0.05, origin_xz[1]], 0.88 + 0.04 * phase.abs(), 0.55),
        BpmMotif::ElasticHop => {
            let hop = (beats * std::f32::consts::PI).sin().abs();
            ([origin_xz[0], 0.35 * hop, origin_xz[1]], 0.84 + 0.32 * hop, 0.45 * hop)
        }
        BpmMotif::MoveToXz { target_xz } => {
            let alpha = (t_s / 10.0).clamp(0.0, 1.0);
            let x = origin_xz[0] + (target_xz[0] - origin_xz[0]) * alpha;
            let z = origin_xz[1] + (target_xz[1] - origin_xz[1]) * alpha;
            ([x, 0.08 * phase.abs(), z], 1.0 + 0.10 * phase, 0.25 * phase)
        }
    };

    let torso = JellySdfCapsule::volume_preserving(
        "torso",
        [root_xyz[0], root_xyz[1] + 0.65, root_xyz[2]],
        [root_xyz[0], root_xyz[1] + 1.35, root_xyz[2]],
        0.16,
        stretch_y,
        0.18,
        0.08,
    );
    let head = JellySdfCapsule::volume_preserving(
        "head",
        [root_xyz[0], root_xyz[1] + 1.48, root_xyz[2]],
        [root_xyz[0], root_xyz[1] + 1.62, root_xyz[2]],
        0.14,
        1.0 + 0.06 * phase,
        0.12,
        0.06,
    );
    let arm_l = JellySdfCapsule::volume_preserving(
        "arm_l",
        [root_xyz[0] - 0.18, root_xyz[1] + 1.25, root_xyz[2]],
        [root_xyz[0] - 0.56, root_xyz[1] + 1.35 + arm_lift, root_xyz[2]],
        0.075,
        1.0 + 0.08 * phase,
        0.14,
        0.05,
    );
    let arm_r = JellySdfCapsule::volume_preserving(
        "arm_r",
        [root_xyz[0] + 0.18, root_xyz[1] + 1.25, root_xyz[2]],
        [root_xyz[0] + 0.56, root_xyz[1] + 1.35 - arm_lift, root_xyz[2]],
        0.075,
        1.0 - 0.08 * phase,
        0.14,
        0.05,
    );

    JellyPoseFrame {
        beat_phase: beats.fract(),
        root_xyz,
        capsules: vec![torso, head, arm_l, arm_r],
        beat_env,
    }
}
