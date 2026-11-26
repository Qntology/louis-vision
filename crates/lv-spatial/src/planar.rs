//! 3D metric plane fitting over Census-SGBM or Metric TIPS depth (`NormalMap` <= 25 deg), physical sizing & anisotropy.

use crate::camera::{Intrinsics, Pose};
use crate::egoexo::{stereo_sigma_z, ViewObs};
use crate::fit3::{M3, V3};
use crate::frame::{DepthKind, DepthMap, NormalMap};
use crate::stereo::StereoRig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceClassHint {
    PaperDocument,
    IndustrialNameplate,
    DisplayScreen,
    GenericPlanar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhysicalFormFactor {
    IsoA4,
    IsoA3,
    UsLetter,
    ShippingLabel100x150,
    HazardLabel100x100,
    IndustrialPlaque,
    Display16x9,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanarCandidateRoi {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
    pub hint: SurfaceClassHint,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanarFitCfg {
    pub min_inliers: usize,
    pub max_samples: usize,
    pub ransac_iters: usize,
    pub max_depth_m: f32,
    pub base_inlier_thresh_m: f32,
    pub sigma_z_mult: f32,
    pub normal_agree_deg: f32,
    pub trim_q: f32,
}

impl Default for PlanarFitCfg {
    fn default() -> Self {
        Self {
            min_inliers: 24,
            max_samples: 420,
            ransac_iters: 72,
            max_depth_m: 8.0,
            base_inlier_thresh_m: 0.004,
            sigma_z_mult: 2.5,
            normal_agree_deg: 25.0,
            trim_q: 0.02,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanarPatch3D {
    pub center_w: V3,
    pub normal_w: V3,
    pub tangent_u_w: V3,
    pub tangent_v_w: V3,
    pub width_mm: f32,
    pub height_mm: f32,
    pub form_factor: PhysicalFormFactor,
    pub hint: SurfaceClassHint,
    pub corners_w: [V3; 4],
    pub corners_px: [[f32; 2]; 4],
    pub effective_px_per_mm_u: f32,
    pub effective_px_per_mm_v: f32,
    pub view_cos: f32,
    pub depth_hole_frac: f32,
    pub plane_rmse_m: f32,
    pub sigma_z_m: f32,
    pub inliers: usize,
    pub view_obs: ViewObs,
}

pub fn fit_planar_patch(
    roi: &PlanarCandidateRoi,
    depth: &DepthMap,
    normals: Option<&NormalMap>,
    intr: &Intrinsics,
    pose: &Pose,
    rig: Option<&StereoRig>,
    cfg: &PlanarFitCfg,
) -> Option<PlanarPatch3D> {
    if depth.kind != DepthKind::Metric || depth.width == 0 || depth.height == 0 {
        return None;
    }
    let iw = intr.width.max(1) as f32;
    let ih = intr.height.max(1) as f32;
    let dw = depth.width as usize;
    let dh = depth.height as usize;

    let gx0 = ((roi.x0 as f32 / iw) * dw as f32).floor().max(0.0) as usize;
    let gy0 = ((roi.y0 as f32 / ih) * dh as f32).floor().max(0.0) as usize;
    let gx1 = (((roi.x1 as f32 / iw) * dw as f32).ceil() as usize).clamp(gx0 + 1, dw);
    let gy1 = (((roi.y1 as f32 / ih) * dh as f32).ceil() as usize).clamp(gy0 + 1, dh);

    let area = (gx1 - gx0) * (gy1 - gy0);
    let stride = (((area as f64 / cfg.max_samples.max(32) as f64).sqrt()).floor() as usize).max(1);
    let scale_x = iw / dw as f32;
    let scale_y = ih / dh as f32;

    let (fwd_c, right_c, up_c) = intr.camera_axes(pose.yaw);

    let mut total_px = 0usize;
    let mut hole_px = 0usize;
    let mut pts: Vec<V3> = Vec::with_capacity(cfg.max_samples + 16);
    let mut world_normals: Vec<Option<V3>> = Vec::new();
    let mut depths_valid: Vec<f32> = Vec::new();

    for gy in gy0..gy1 {
        for gx in gx0..gx1 {
            total_px += 1;
            let d = depth.data[gy * dw + gx];
            if !d.is_finite() {
                hole_px += 1;
                continue;
            }
            if d < 0.05 || d > cfg.max_depth_m {
                continue;
            }
            if ((gx - gx0) % stride != 0) || ((gy - gy0) % stride != 0) {
                continue;
            }
            let px = (gx as f32 + 0.5) * scale_x;
            let py = (gy as f32 + 0.5) * scale_y;
            pts.push(intr.unproject_optical(pose, px, py, d));
            depths_valid.push(d);

            if let Some(nm) = normals {
                world_normals.push(sample_world_normal(nm, gx, gy, dw, dh, fwd_c, right_c, up_c));
            }
        }
    }

    if pts.len() < cfg.min_inliers {
        return None;
    }

    depths_valid.sort_by(|a, b| a.total_cmp(b));
    let d_med = depths_valid[depths_valid.len() / 2];
    let sigma_z_m = if let Some(r) = rig {
        let f_grid_px = r.focal_on(intr.width.max(1), depth.width.max(1));
        stereo_sigma_z(
            d_med as f64,
            f_grid_px as f64,
            r.baseline_m as f64,
            r.subpixel_px as f64,
        ) as f32
    } else {
        0.015 * d_med * d_med
    };
    let inlier_thresh = cfg.base_inlier_thresh_m.max(cfg.sigma_z_mult * sigma_z_m) as f64;

    let origin_w: V3 = [pose.x as f64, pose.eye_m as f64, pose.z as f64];
    let norm_slice = normals.map(|_| world_normals.as_slice());
    let (inliers, mut normal_w, rmse_m) = ransac_fit_plane(&pts, norm_slice, inlier_thresh, cfg)?;

    let c_inliers = mean_v3(&pts, &inliers);
    let to_cam = sub3(origin_w, c_inliers);
    if dot3(normal_w, to_cam) < 0.0 {
        normal_w = neg3(normal_w);
    }

    let (u_w, v_w) = canonical_reading_axes(&pts, &inliers, c_inliers, normal_w, intr, pose, cfg.trim_q);
    let grid_step_m = ((stride as f64 * scale_x as f64) * d_med as f64) / intr.focal_px();
    let (center_w, width_mm, height_mm, corners_w) =
        trimmed_plane_bounds(&pts, &inliers, c_inliers, u_w, v_w, cfg.trim_q, grid_step_m)?;

    let mut corners_px = [[0.0f32; 2]; 4];
    for (k, &cw) in corners_w.iter().enumerate() {
        let (px, py, z) = intr.project_world_unclamped(pose, cw)?;
        if z <= 0.02 {
            return None;
        }
        corners_px[k] = [px, py];
    }

    let (px_u, px_v) = effective_density(&corners_px, width_mm, height_mm);
    let center_dist = norm3(sub3(center_w, origin_w)).max(1e-6);
    let ray_to_cam = scale3(sub3(origin_w, center_w), 1.0 / center_dist);
    let view_cos = dot3(normal_w, ray_to_cam).clamp(-1.0, 1.0) as f32;
    let form_factor = classify_physical_form_factor(width_mm, height_mm, roi.hint);

    Some(PlanarPatch3D {
        center_w,
        normal_w,
        tangent_u_w: u_w,
        tangent_v_w: v_w,
        width_mm,
        height_mm,
        form_factor,
        hint: roi.hint,
        corners_w,
        corners_px,
        effective_px_per_mm_u: px_u,
        effective_px_per_mm_v: px_v,
        view_cos,
        depth_hole_frac: hole_px as f32 / total_px.max(1) as f32,
        plane_rmse_m: rmse_m,
        sigma_z_m,
        inliers: inliers.len(),
        view_obs: ViewObs {
            origin: origin_w,
            point: center_w,
            sigma_z: (sigma_z_m as f64).max(rmse_m as f64).max(1e-3),
            sigma_theta: 1.5 / intr.focal_px().max(1.0),
            weight: (inliers.len() as f64 / cfg.min_inliers.max(1) as f64).clamp(1.0, 8.0),
        },
    })
}

fn sample_world_normal(
    nm: &NormalMap,
    gx: usize,
    gy: usize,
    dw: usize,
    dh: usize,
    fwd_c: [f32; 3],
    right_c: [f32; 3],
    up_c: [f32; 3],
) -> Option<V3> {
    if nm.width == 0 || nm.height == 0 {
        return None;
    }
    let nw = nm.width as usize;
    let nh = nm.height as usize;
    let nx = ((gx * nw) / dw.max(1)).min(nw - 1);
    let ny = ((gy * nh) / dh.max(1)).min(nh - 1);
    let [vx, vy, vz] = *nm.data.get(ny * nw + nx)?;
    if !vx.is_finite() || !vy.is_finite() || !vz.is_finite() {
        return None;
    }
    let w = [
        (vx * right_c[0] + vy * (-up_c[0]) + vz * fwd_c[0]) as f64,
        (vx * right_c[1] + vy * (-up_c[1]) + vz * fwd_c[1]) as f64,
        (vx * right_c[2] + vy * (-up_c[2]) + vz * fwd_c[2]) as f64,
    ];
    unit3(w)
}

fn classify_physical_form_factor(
    width_mm: f32,
    height_mm: f32,
    hint: SurfaceClassHint,
) -> PhysicalFormFactor {
    let short = width_mm.min(height_mm).max(1e-3);
    let long = width_mm.max(height_mm).max(1e-3);
    let rel_match = |s_ref: f32, l_ref: f32, tol: f32| -> bool {
        ((short - s_ref).abs() / s_ref <= tol) && ((long - l_ref).abs() / l_ref <= tol)
    };

    if matches!(hint, SurfaceClassHint::DisplayScreen) {
        let aspect = long / short;
        let diag = short.hypot(long);
        if (1.55..=2.05).contains(&aspect) && (250.0..=1400.0).contains(&diag) {
            return PhysicalFormFactor::Display16x9;
        }
    }
    if rel_match(210.0, 297.0, 0.12) {
        PhysicalFormFactor::IsoA4
    } else if rel_match(297.0, 420.0, 0.12) {
        PhysicalFormFactor::IsoA3
    } else if rel_match(216.0, 279.0, 0.10) {
        PhysicalFormFactor::UsLetter
    } else if rel_match(100.0, 150.0, 0.12) {
        PhysicalFormFactor::ShippingLabel100x150
    } else if rel_match(100.0, 100.0, 0.12) {
        PhysicalFormFactor::HazardLabel100x100
    } else if (1.62..=1.92).contains(&(long / short)) && (300.0..=1250.0).contains(&short.hypot(long))
    {
        PhysicalFormFactor::Display16x9
    } else if (28.0..=125.0).contains(&short) && (55.0..=265.0).contains(&long) {
        PhysicalFormFactor::IndustrialPlaque
    } else {
        PhysicalFormFactor::Custom
    }
}

#[inline]
fn add3(a: V3, b: V3) -> V3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
#[inline]
fn sub3(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
#[inline]
fn neg3(a: V3) -> V3 { [-a[0], -a[1], -a[2]] }
#[inline]
fn scale3(a: V3, s: f64) -> V3 { [a[0] * s, a[1] * s, a[2] * s] }
#[inline]
fn dot3(a: V3, b: V3) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
#[inline]
fn cross3(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
#[inline]
fn norm3(a: V3) -> f64 { dot3(a, a).sqrt() }
#[inline]
fn unit3(a: V3) -> Option<V3> {
    let n = norm3(a);
    (n.is_finite() && n > 1e-12).then(|| scale3(a, 1.0 / n))
}

fn mean_v3(pts: &[V3], indices: &[usize]) -> V3 {
    let mut acc = [0.0; 3];
    for &i in indices {
        acc[0] += pts[i][0];
        acc[1] += pts[i][1];
        acc[2] += pts[i][2];
    }
    let inv = 1.0 / indices.len().max(1) as f64;
    scale3(acc, inv)
}

fn ransac_fit_plane(
    pts: &[V3],
    world_normals: Option<&[Option<V3>]>,
    inlier_thresh: f64,
    cfg: &PlanarFitCfg,
) -> Option<(Vec<usize>, V3, f32)> {
    let n = pts.len();
    if n < 3 {
        return None;
    }
    let cos_agree = (cfg.normal_agree_deg as f64).to_radians().cos();
    let mut best_inliers: Vec<usize> = Vec::new();
    let mut best_score = f64::NEG_INFINITY;

    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15 ^ (n as u64);
    let mut next_idx = || -> usize {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed as usize) % n
    };

    for _iter in 0..cfg.ransac_iters {
        let i0 = next_idx();
        let i1 = next_idx();
        let i2 = next_idx();
        if i0 == i1 || i1 == i2 || i0 == i2 {
            continue;
        }
        let e1 = sub3(pts[i1], pts[i0]);
        let e2 = sub3(pts[i2], pts[i0]);
        let Some(n_try) = unit3(cross3(e1, e2)) else { continue };

        if let Some(nms) = world_normals {
            let ok = [i0, i1, i2].into_iter().all(|idx| {
                nms.get(idx)
                    .and_then(|x| *x)
                    .map_or(true, |nw| dot3(nw, n_try).abs() >= cos_agree)
            });
            if !ok {
                continue;
            }
        }

        let d0 = -dot3(n_try, pts[i0]);
        let mut cur_inliers = Vec::new();
        let mut huber_score = 0.0f64;
        for (idx, p) in pts.iter().enumerate() {
            let dist = (dot3(n_try, *p) + d0).abs();
            if dist <= inlier_thresh {
                if let Some(nms) = world_normals {
                    if let Some(Some(nw)) = nms.get(idx) {
                        if dot3(*nw, n_try).abs() < cos_agree {
                            continue;
                        }
                    }
                }
                cur_inliers.push(idx);
                huber_score += 1.0 - 0.5 * (dist / inlier_thresh).powi(2);
            }
        }
        if huber_score > best_score && cur_inliers.len() >= best_inliers.len() {
            best_score = huber_score;
            best_inliers = cur_inliers;
        }
    }

    if best_inliers.len() < cfg.min_inliers {
        return None;
    }

    let (n_ref1, _) = pca_plane_normal(pts, &best_inliers)?;
    let c_ref1 = mean_v3(pts, &best_inliers);
    let d_ref1 = -dot3(n_ref1, c_ref1);
    let refined_inliers: Vec<usize> = pts
        .iter()
        .enumerate()
        .filter_map(|(i, p)| ((dot3(n_ref1, *p) + d_ref1).abs() <= inlier_thresh).then_some(i))
        .collect();
    let final_inliers = if refined_inliers.len() >= best_inliers.len() {
        refined_inliers
    } else {
        best_inliers
    };

    let (normal_final, rmse_m) = pca_plane_normal(pts, &final_inliers)?;
    Some((final_inliers, normal_final, rmse_m as f32))
}

fn pca_plane_normal(pts: &[V3], inliers: &[usize]) -> Option<(V3, f64)> {
    if inliers.len() < 3 {
        return None;
    }
    let mu = mean_v3(pts, inliers);
    let mut cov: M3 = [[0.0; 3]; 3];
    for &idx in inliers {
        let d = sub3(pts[idx], mu);
        for r in 0..3 {
            for c in 0..3 {
                cov[r][c] += d[r] * d[c];
            }
        }
    }
    let inv_n = 1.0 / inliers.len() as f64;
    for r in 0..3 {
        for c in 0..3 {
            cov[r][c] *= inv_n;
        }
    }

    let mut a = cov;
    let mut v: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _sweep in 0..12 {
        for &(p, q) in &[(0usize, 1usize), (0, 2), (1, 2)] {
            let apq = a[p][q];
            if apq.abs() <= 1e-15 {
                continue;
            }
            let tau = (a[q][q] - a[p][p]) / (2.0 * apq);
            let t = if tau >= 0.0 {
                1.0 / (tau + (1.0 + tau * tau).sqrt())
            } else {
                -1.0 / (-tau + (1.0 + tau * tau).sqrt())
            };
            let c = 1.0 / (1.0 + t * t).sqrt();
            let s = t * c;

            a[p][p] -= t * apq;
            a[q][q] += t * apq;
            a[p][q] = 0.0;
            a[q][p] = 0.0;

            let r = 3 - p - q;
            let (arp, arq) = (a[r][p], a[r][q]);
            a[r][p] = c * arp - s * arq;
            a[p][r] = a[r][p];
            a[r][q] = s * arp + c * arq;
            a[q][r] = a[r][q];

            for k in 0..3 {
                let (vkp, vkq) = (v[k][p], v[k][q]);
                v[k][p] = c * vkp - s * vkq;
                v[k][q] = s * vkp + c * vkq;
            }
        }
    }

    let mut order = [0usize, 1, 2];
    order.sort_by(|&i, &j| a[i][i].total_cmp(&a[j][j]));
    let lambda_min = a[order[0]][order[0]].max(0.0);
    let lambda_mid = a[order[1]][order[1]].max(0.0);
    if lambda_mid < 1.44e-4 {
        return None;
    }
    let min_col = order[0];
    let n_vec = unit3([v[0][min_col], v[1][min_col], v[2][min_col]])?;
    Some((n_vec, lambda_min.sqrt()))
}

fn canonical_reading_axes(
    pts: &[V3],
    inliers: &[usize],
    mean_w: V3,
    normal_w: V3,
    intr: &Intrinsics,
    pose: &Pose,
    trim_q: f32,
) -> (V3, V3) {
    let (_, right_c, up_c) = intr.camera_axes(pose.yaw);
    let right_w: V3 = [right_c[0] as f64, right_c[1] as f64, right_c[2] as f64];
    let cam_down_w: V3 = [-up_c[0] as f64, -up_c[1] as f64, -up_c[2] as f64];

    let grav: V3 = [0.0, -1.0, 0.0];
    let g_proj = sub3(grav, scale3(normal_w, dot3(grav, normal_w)));
    let c_proj = sub3(cam_down_w, scale3(normal_w, dot3(cam_down_w, normal_w)));

    let mut v0 = if norm3(g_proj) >= 0.25 && !(normal_w[1].abs() > 0.85 && dot3(g_proj, cam_down_w) < 0.0) {
        unit3(g_proj).unwrap_or([0.0, -1.0, 0.0])
    } else {
        unit3(c_proj).or_else(|| unit3(g_proj)).unwrap_or([1.0, 0.0, 0.0])
    };

    let mut u0 = unit3(cross3(normal_w, v0)).unwrap_or(right_w);
    if dot3(u0, right_w) < 0.0 {
        u0 = neg3(u0);
    }
    v0 = unit3(sub3(v0, scale3(u0, dot3(v0, u0)))).unwrap_or(v0);

    let uv0: Vec<(f64, f64)> = inliers
        .iter()
        .map(|&i| {
            let rel = sub3(pts[i], mean_w);
            (dot3(rel, u0), dot3(rel, v0))
        })
        .collect();

    let q = (trim_q as f64).clamp(0.0, 0.20);
    let mut u_buf = vec![0.0; uv0.len()];
    let mut v_buf = vec![0.0; uv0.len()];
    let area_at = |deg: f64, ub: &mut [f64], vb: &mut [f64]| -> f64 {
        let (s, c) = deg.to_radians().sin_cos();
        for (k, &(u, v)) in uv0.iter().enumerate() {
            ub[k] = c * u + s * v;
            vb[k] = -s * u + c * v;
        }
        ub.sort_by(|a, b| a.total_cmp(b));
        vb.sort_by(|a, b| a.total_cmp(b));
        let lo = ((ub.len() - 1) as f64 * q).round() as usize;
        let hi = ((ub.len() - 1) as f64 * (1.0 - q)).round() as usize;
        (ub[hi] - ub[lo]).max(1e-4) * (vb[hi] - vb[lo]).max(1e-4)
    };

    let mut best_deg = 0.0f64;
    let mut best_area = area_at(0.0, &mut u_buf, &mut v_buf);
    for step in -44..=45 {
        let deg = step as f64;
        let ar = area_at(deg, &mut u_buf, &mut v_buf);
        if ar < best_area * 0.999 {
            best_area = ar;
            best_deg = deg;
        }
    }
    for sub in -4..=4 {
        let deg = (best_deg + sub as f64 * 0.25).clamp(-44.9, 45.0);
        let ar = area_at(deg, &mut u_buf, &mut v_buf);
        if ar < best_area {
            best_area = ar;
            best_deg = deg;
        }
    }

    let (s, c) = best_deg.to_radians().sin_cos();
    let u_final = unit3(add3(scale3(u0, c), scale3(v0, s))).unwrap_or(u0);
    let v_final = unit3(add3(scale3(u0, -s), scale3(v0, c))).unwrap_or(v0);
    (u_final, v_final)
}

fn trimmed_plane_bounds(
    pts: &[V3],
    inliers: &[usize],
    mean_w: V3,
    u_w: V3,
    v_w: V3,
    trim_q: f32,
    grid_step_m: f64,
) -> Option<(V3, f32, f32, [V3; 4])> {
    if inliers.len() < 4 {
        return None;
    }
    let mut us: Vec<f64> = Vec::with_capacity(inliers.len());
    let mut vs: Vec<f64> = Vec::with_capacity(inliers.len());
    for &idx in inliers {
        let rel = sub3(pts[idx], mean_w);
        us.push(dot3(rel, u_w));
        vs.push(dot3(rel, v_w));
    }
    us.sort_by(|a, b| a.total_cmp(b));
    vs.sort_by(|a, b| a.total_cmp(b));

    let q = (trim_q as f64).clamp(0.0, 0.20);
    let unbiased = 1.0 / (1.0 - 2.0 * q).max(0.2);
    let lo = ((inliers.len() - 1) as f64 * q).round() as usize;
    let hi = ((inliers.len() - 1) as f64 * (1.0 - q)).round() as usize;

    let u_mid = 0.5 * (us[lo] + us[hi]);
    let v_mid = 0.5 * (vs[lo] + vs[hi]);
    let full_u = us[inliers.len() - 1] - us[0];
    let full_v = vs[inliers.len() - 1] - vs[0];
    let span_u_m = (((us[hi] - us[lo]) * unbiased).min(full_u) + grid_step_m.min(0.008)).max(0.010);
    let span_v_m = (((vs[hi] - vs[lo]) * unbiased).min(full_v) + grid_step_m.min(0.008)).max(0.010);

    let center_w = add3(mean_w, add3(scale3(u_w, u_mid), scale3(v_w, v_mid)));
    let hu = 0.5 * span_u_m;
    let hv = 0.5 * span_v_m;
    let corners_w = [
        add3(center_w, add3(scale3(u_w, -hu), scale3(v_w, -hv))),
        add3(center_w, add3(scale3(u_w, hu), scale3(v_w, -hv))),
        add3(center_w, add3(scale3(u_w, hu), scale3(v_w, hv))),
        add3(center_w, add3(scale3(u_w, -hu), scale3(v_w, hv))),
    ];

    Some((
        center_w,
        (span_u_m * 1000.0) as f32,
        (span_v_m * 1000.0) as f32,
        corners_w,
    ))
}

fn effective_density(corners_px: &[[f32; 2]; 4], width_mm: f32, height_mm: f32) -> (f32, f32) {
    let dist = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    let top_px = dist(corners_px[0], corners_px[1]);
    let bot_px = dist(corners_px[3], corners_px[2]);
    let left_px = dist(corners_px[0], corners_px[3]);
    let right_px = dist(corners_px[1], corners_px[2]);
    (
        0.5 * (top_px + bot_px) / width_mm.max(1.0),
        0.5 * (left_px + right_px) / height_mm.max(1.0),
    )
}
