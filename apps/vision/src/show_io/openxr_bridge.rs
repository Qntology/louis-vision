//! OpenXR `STAGE` reference space anchor mapping performer 3D tracks & planar callouts to HMD overlays.

use lv_spatial::fit3::{Similarity, V3};
use lv_spatial::zkey::PerformerTrack3D;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XrSandboxAnchorPose {
    pub track_id: u32,
    pub position_m: [f32; 3],
    pub velocity_mps: [f32; 3],
    pub orientation_xyzw: [f32; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XrPlanarCalloutAnchor {
    pub surface_id: u32,
    pub field: String,
    pub text: String,
    pub position_m: [f32; 3],
    pub orientation_xyzw: [f32; 4],
    pub physical_size_mm: [f32; 2],
    pub verified: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XrSandboxAnchorFrame {
    pub reference_space: String,
    pub frame_index: u64,
    pub anchors: Vec<XrSandboxAnchorPose>,
    #[serde(default)]
    pub planar_callouts: Vec<XrPlanarCalloutAnchor>,
}

/// Converts planar right-handed frame axes (`u_raw`, `up_raw`, `n_raw`) into a normalized `SO(3)` `[x,y,z,w]` quaternion.
pub fn basis_to_unit_quat(u_raw: V3, up_raw: V3, n_raw: V3) -> [f32; 4] {
    let norm = |a: V3| (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    let unit = |a: V3, fb: V3| {
        let l = norm(a);
        if l > 1e-12 && l.is_finite() {
            [a[0] / l, a[1] / l, a[2] / l]
        } else {
            fb
        }
    };
    let dot = |a: V3, b: V3| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: V3, b: V3| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };

    let z_ax = unit(n_raw, [0.0, 0.0, 1.0]);
    let up_proj = [
        up_raw[0] - dot(up_raw, z_ax) * z_ax[0],
        up_raw[1] - dot(up_raw, z_ax) * z_ax[1],
        up_raw[2] - dot(up_raw, z_ax) * z_ax[2],
    ];
    let y_ax = unit(up_proj, [0.0, 1.0, 0.0]);
    let mut x_ax = unit(cross(y_ax, z_ax), [1.0, 0.0, 0.0]);
    let z_final = if dot(x_ax, u_raw) < 0.0 {
        x_ax = [-x_ax[0], -x_ax[1], -x_ax[2]];
        [-z_ax[0], -z_ax[1], -z_ax[2]]
    } else {
        z_ax
    };

    let (m00, m01, m02) = (x_ax[0], y_ax[0], z_final[0]);
    let (m10, m11, m12) = (x_ax[1], y_ax[1], z_final[1]);
    let (m20, m21, m22) = (x_ax[2], y_ax[2], z_final[2]);
    let tr = m00 + m11 + m22;
    let (qx, qy, qz, qw) = if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        ((m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, 0.25 * s)
    } else if m00 > m11 && m00 > m22 {
        let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
        (0.25 * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s)
    } else if m11 > m22 {
        let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
        ((m01 + m10) / s, 0.25 * s, (m12 + m21) / s, (m02 - m20) / s)
    } else {
        let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
        ((m02 + m20) / s, (m12 + m21) / s, 0.25 * s, (m10 - m01) / s)
    };
    let qn = (qx * qx + qy * qy + qz * qz + qw * qw).sqrt().max(1e-12);
    [
        (qx / qn) as f32,
        (qy / qn) as f32,
        (qz / qn) as f32,
        (qw / qn) as f32,
    ]
}

/// Projects a rig-space planar patch pose via `Similarity { s, r, t }` into OpenXR `STAGE` `[pos_m, xyzw]`.
pub fn transform_planar_pose_to_stage(
    rig_to_stage: &Similarity,
    center_rig: V3,
    u_rig: V3,
    v_rig: V3,
    n_rig: V3,
) -> ([f32; 3], [f32; 4]) {
    let rot = |v: V3| -> V3 {
        let r = &rig_to_stage.r;
        [
            r[0][0] * v[0] + r[0][1] * v[1] + r[0][2] * v[2],
            r[1][0] * v[0] + r[1][1] * v[1] + r[1][2] * v[2],
            r[2][0] * v[0] + r[2][1] * v[1] + r[2][2] * v[2],
        ]
    };
    let s = rig_to_stage.s;
    let rc = rot(center_rig);
    let pos_stage = [
        (s * rc[0] + rig_to_stage.t[0]) as f32,
        (s * rc[1] + rig_to_stage.t[1]) as f32,
        (s * rc[2] + rig_to_stage.t[2]) as f32,
    ];
    let up_rig = [-v_rig[0], -v_rig[1], -v_rig[2]];
    (pos_stage, basis_to_unit_quat(rot(u_rig), rot(up_rig), rot(n_rig)))
}

/// Projects metric sandbox tracks directly into OpenXR `STAGE` coordinates (no local origin drift).
pub fn map_to_openxr_sandbox_space(frame_index: u64, performers: &[PerformerTrack3D]) -> XrSandboxAnchorFrame {
    let anchors = performers
        .iter()
        .map(|p| XrSandboxAnchorPose {
            track_id: p.id,
            position_m: p.pos_m,
            velocity_mps: p.vel_mps,
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        })
        .collect();
    XrSandboxAnchorFrame {
        reference_space: "STAGE".into(),
        frame_index,
        anchors,
        planar_callouts: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_sandbox_performers_and_planar_callouts_to_openxr_stage_space() {
        let p = PerformerTrack3D {
            id: 4,
            pos_m: [1.0, 1.2, -3.4],
            vel_mps: [0.2, 0.0, 0.1],
            bbox_px: [10, 20, 30, 40],
            pixel_count: 100,
            missed_frames: 0,
        };
        let mut xr = map_to_openxr_sandbox_space(9, &[p]);
        assert_eq!(xr.reference_space, "STAGE");
        assert_eq!(xr.anchors.len(), 1);
        assert_eq!(xr.anchors[0].position_m, [1.0, 1.2, -3.4]);

        let sim = Similarity {
            s: 1.0,
            r: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            t: [0.5, 0.0, 1.0],
        };
        let (pos, q) = transform_planar_pose_to_stage(
            &sim,
            [0.2, 1.4, 0.8],
            [1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
        );
        let qn = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        assert!((qn - 1.0).abs() < 1e-5);
        assert!((pos[0] - 0.7).abs() < 1e-5 && (pos[2] - 1.8).abs() < 1e-5);
        xr.planar_callouts.push(XrPlanarCalloutAnchor {
            surface_id: 1,
            field: "kks_code".into(),
            text: "10LAB10AP001".into(),
            position_m: pos,
            orientation_xyzw: q,
            physical_size_mm: [180.0, 120.0],
            verified: true,
        });
        let raw = serde_json::to_string(&xr).unwrap();
        let rt: XrSandboxAnchorFrame = serde_json::from_str(&raw).unwrap();
        assert_eq!(rt.planar_callouts.len(), 1);
    }
}
