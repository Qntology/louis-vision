//! H^-1 + planar glare-hole completion back-projection and multi-view `egoexo::fuse_3d` persistent callouts.

use lv_spatial::camera::{Intrinsics, Pose};
use lv_spatial::egoexo::{fuse_3d, Fused3D, ViewObs};
use lv_spatial::fit3::V3;
use lv_spatial::frame::DepthMap;
use lv_spatial::planar::PlanarPatch3D;
use serde::{Deserialize, Serialize};

use crate::homography::PlanarHomography;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnchoredField3D {
    pub category: String,
    pub field: String,
    pub text_repaired: String,
    pub rect_bbox_px: (u32, u32, u32, u32),
    pub image_quad_px: [[f32; 2]; 4],
    pub center_w: V3,
    pub corners_w: [V3; 4],
    pub view_obs: ViewObs,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectedAnchorCallout {
    pub key: String,
    pub text_repaired: String,
    pub center_w: V3,
    pub sigma_m: f32,
    pub image_quad_px: [[f32; 2]; 4],
}

pub fn backproject_rect_point(
    h: &PlanarHomography,
    parent: &PlanarPatch3D,
    u_px: f32,
    v_px: f32,
    depth: Option<&DepthMap>,
    intr: &Intrinsics,
    pose: &Pose,
) -> (V3, [f32; 2]) {
    let (uf, vf) = (u_px as f64, v_px as f64);
    let plane_pt: V3 = [
        h.origin_tl_w[0] + uf * h.step_u_w[0] + vf * h.step_v_w[0],
        h.origin_tl_w[1] + uf * h.step_u_w[1] + vf * h.step_v_w[1],
        h.origin_tl_w[2] + uf * h.step_u_w[2] + vf * h.step_v_w[2],
    ];
    let Some((px_f, py_f, z_pred)) = h.rect_to_img(uf, vf) else {
        return (plane_pt, [0.0, 0.0]);
    };
    let (px, py) = (px_f as f32, py_f as f32);

    if let Some(dm) = depth {
        let d_opt = dm.at_scaled(px, py, intr.width, intr.height);
        let tol = (2.0 * parent.sigma_z_m).max(0.015);
        if d_opt.is_finite() && (d_opt - z_pred as f32).abs() <= tol {
            return (intr.unproject_optical(pose, px, py, d_opt), [px, py]);
        }
    }
    (plane_pt, [px, py])
}

#[derive(Clone, Debug, Default)]
pub struct PlanarAnchorRegistry {
    entries: Vec<AnchorTrack>,
}

#[derive(Clone, Debug)]
struct AnchorTrack {
    key: String,
    text_repaired: String,
    half_offsets_w: [V3; 4],
    obs_ring: Vec<ViewObs>,
    fused: Fused3D,
}

impl PlanarAnchorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn upsert(&mut self, field: &AnchoredField3D, max_obs: usize) -> Fused3D {
        let key = format!("{}:{}", field.category, field.field);
        let cap = max_obs.clamp(1, 32);
        let offsets = field.corners_w.map(|cw| {
            [
                cw[0] - field.center_w[0],
                cw[1] - field.center_w[1],
                cw[2] - field.center_w[2],
            ]
        });

        if let Some(track) = self.entries.iter_mut().find(|e| {
            e.key == key && {
                let d = (e.fused.point[0] - field.center_w[0]).powi(2)
                    + (e.fused.point[1] - field.center_w[1]).powi(2)
                    + (e.fused.point[2] - field.center_w[2]).powi(2);
                d.sqrt() < 0.35
            }
        }) {
            if track.obs_ring.len() >= cap {
                track.obs_ring.remove(0);
            }
            track.obs_ring.push(field.view_obs);
            track.text_repaired = field.text_repaired.clone();
            track.half_offsets_w = offsets;
            if let Some(f) = fuse_3d(&track.obs_ring) {
                track.fused = f;
            }
            return track.fused;
        }

        let fused = fuse_3d(std::slice::from_ref(&field.view_obs)).unwrap_or(Fused3D {
            point: field.center_w,
            cov: [[1e-3, 0.0, 0.0], [0.0, 1e-3, 0.0], [0.0, 0.0, 1e-3]],
            sigma_m: field.view_obs.sigma_z as f32,
        });
        self.entries.push(AnchorTrack {
            key,
            text_repaired: field.text_repaired.clone(),
            half_offsets_w: offsets,
            obs_ring: vec![field.view_obs],
            fused,
        });
        fused
    }

    pub fn project_active(&self, intr: &Intrinsics, pose: &Pose) -> Vec<ProjectedAnchorCallout> {
        let mut out = Vec::with_capacity(self.entries.len());
        for tr in &self.entries {
            if intr.project_world(pose, tr.fused.point).is_none() {
                continue;
            }
            let mut quad = [[0.0f32; 2]; 4];
            let mut valid = true;
            for (i, off) in tr.half_offsets_w.iter().enumerate() {
                let cw = [
                    tr.fused.point[0] + off[0],
                    tr.fused.point[1] + off[1],
                    tr.fused.point[2] + off[2],
                ];
                if let Some((px, py, _)) = intr.project_world_unclamped(pose, cw) {
                    quad[i] = [px, py];
                } else {
                    valid = false;
                    break;
                }
            }
            if valid {
                out.push(ProjectedAnchorCallout {
                    key: tr.key.clone(),
                    text_repaired: tr.text_repaired.clone(),
                    center_w: tr.fused.point,
                    sigma_m: tr.fused.sigma_m,
                    image_quad_px: quad,
                });
            }
        }
        out
    }
}
