//! Ground-plane analytical depth, IRWLS scale/disparity alignment, and contact-row drop range.

use serde::{Deserialize, Serialize};

use crate::camera::{Intrinsics, Pose};
use crate::frame::{DepthKind, DepthMap, Mask, NormalMap};

pub type HeightFn<'a> = &'a dyn Fn(f32, f32) -> f32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignSpace {
    Scale,
    #[default]
    Disparity,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlignCfg {
    pub space: AlignSpace,
    pub max_range_m: f32,
    pub clip_to_support: bool,
    pub min_samples: usize,
    pub iters: usize,
}

impl Default for AlignCfg {
    fn default() -> Self {
        Self {
            space: AlignSpace::Disparity,
            max_range_m: 12.0,
            clip_to_support: true,
            min_samples: 24,
            iters: 8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlignFit {
    pub space: AlignSpace,
    pub a: f32,
    pub b: f32,
    pub clip_above_m: f32,
    pub inlier_share: f32,
    pub median_rel_err: f32,
}

impl AlignFit {
    pub fn apply_one(&self, d: f32) -> f32 {
        if !d.is_finite() || d <= 0.0 {
            return f32::INFINITY;
        }
        let aligned = match self.space {
            AlignSpace::Scale => d * self.a,
            AlignSpace::Disparity => {
                let disp = self.a / d + self.b;
                if disp <= 1e-6 {
                    f32::INFINITY
                } else {
                    1.0 / disp
                }
            }
        };
        if !aligned.is_finite() || aligned > self.clip_above_m {
            f32::INFINITY
        } else {
            aligned
        }
    }
}

pub fn ground_depth(
    intr: &Intrinsics,
    pose: &Pose,
    px: f32,
    py: f32,
    height: Option<HeightFn<'_>>,
    max_range_m: f32,
) -> Option<f32> {
    let r = intr.ray(pose.yaw, px, py);
    if r[1] >= -1e-4 {
        return None;
    }
    let mut t = -pose.eye_m / r[1];
    if let Some(hfn) = height {
        for _ in 0..5 {
            let wx = pose.x + t * r[0];
            let wz = pose.z + t * r[2];
            let gz = hfn(wx, wz);
            let next = (gz - pose.eye_m) / r[1];
            if !next.is_finite() || next <= 0.0 {
                return None;
            }
            if (next - t).abs() < 1e-3 {
                t = next;
                break;
            }
            t = next;
        }
    }
    if t.is_finite() && t > 0.0 && t <= max_range_m {
        Some(t)
    } else {
        None
    }
}

pub const GROUND_SEED_DEG: f32 = 60.0;

pub fn faces_up(n: [f32; 3], up: [f32; 3], max_deg: f32) -> bool {
    let dot = n[0] * up[0] + n[1] * up[1] + n[2] * up[2];
    let n_len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-8);
    let u_len = (up[0] * up[0] + up[1] * up[1] + up[2] * up[2]).sqrt().max(1e-8);
    (dot / (n_len * u_len)) >= max_deg.to_radians().cos()
}

pub fn ground_normal(
    depth: &DepthMap,
    normals: &NormalMap,
    intr: &Intrinsics,
    pose: &Pose,
    height: Option<HeightFn<'_>>,
    px: f32,
    py: f32,
) -> Option<[f32; 3]> {
    let _ = (depth, pose, height);
    let up = intr.world_up_in_camera();
    let n = normals.at_scaled(px, py, intr.width, intr.height);
    faces_up(n, up, GROUND_SEED_DEG).then_some(n)
}

pub fn fit_ground(
    depth: &DepthMap,
    intr: &Intrinsics,
    pose: &Pose,
    height: Option<HeightFn<'_>>,
    raw_cap_m: Option<f32>,
    cfg: &AlignCfg,
) -> Option<AlignFit> {
    fit_ground_with(depth, None, intr, pose, height, raw_cap_m, cfg)
}

pub fn fit_ground_with(
    depth: &DepthMap,
    normals: Option<&NormalMap>,
    intr: &Intrinsics,
    pose: &Pose,
    height: Option<HeightFn<'_>>,
    raw_cap_m: Option<f32>,
    cfg: &AlignCfg,
) -> Option<AlignFit> {
    let step_x = (depth.width / 48).max(1);
    let step_y = (depth.height / 32).max(1);
    let mut pairs: Vec<(f32, f32)> = Vec::new();
    let up = intr.world_up_in_camera();

    let mut y = 0;
    while y < depth.height {
        let mut x = 0;
        while x < depth.width {
            let raw = depth.at(x, y);
            if raw.is_finite() && raw > 0.0 && raw_cap_m.is_none_or(|c| raw < c * 0.99) {
                let px = (x as f32 + 0.5) / depth.width.max(1) as f32 * intr.width as f32;
                let py = (y as f32 + 0.5) / depth.height.max(1) as f32 * intr.height as f32;
                if let Some(nm) = normals {
                    let n = nm.at_scaled(px, py, intr.width, intr.height);
                    if !faces_up(n, up, GROUND_SEED_DEG) {
                        x += step_x;
                        continue;
                    }
                }
                if let Some(gt) = ground_depth(intr, pose, px, py, height, cfg.max_range_m) {
                    pairs.push((raw, gt));
                }
            }
            x += step_x;
        }
        y += step_y;
    }

    if pairs.len() < cfg.min_samples {
        return None;
    }

    let mut a = 1.0f64;
    let mut b = 0.0f64;
    let mut weights = vec![1.0f64; pairs.len()];
    for _ in 0..cfg.iters {
        match cfg.space {
            AlignSpace::Scale => {
                let (mut num, mut den) = (0.0f64, 0.0f64);
                for (&(raw, gt), &w) in pairs.iter().zip(&weights) {
                    let r = raw as f64;
                    num += w * r * gt as f64;
                    den += w * r * r;
                }
                a = if den > 1e-12 { num / den } else { 1.0 };
                b = 0.0;
            }
            AlignSpace::Disparity => {
                let (mut s00, mut s01, mut s11, mut t0, mut t1) = (0.0, 0.0, 0.0, 0.0, 0.0);
                for (&(raw, gt), &w) in pairs.iter().zip(&weights) {
                    let x = 1.0 / (raw as f64).max(1e-4);
                    let y = 1.0 / (gt as f64).max(1e-4);
                    s00 += w * x * x;
                    s01 += w * x;
                    s11 += w;
                    t0 += w * x * y;
                    t1 += w * y;
                }
                let det = s00 * s11 - s01 * s01;
                if det.abs() <= 1e-18 {
                    return None;
                }
                a = (s11 * t0 - s01 * t1) / det;
                b = (-s01 * t0 + s00 * t1) / det;
            }
        }
        let mut errs: Vec<f64> = pairs
            .iter()
            .map(|&(raw, gt)| match cfg.space {
                AlignSpace::Scale => ((a * raw as f64) - gt as f64).abs() / (gt as f64).max(1e-4),
                AlignSpace::Disparity => ((a / raw as f64 + b) - 1.0 / gt as f64).abs(),
            })
            .collect();
        let mut sorted = errs.clone();
        sorted.sort_by(f64::total_cmp);
        let scale = (sorted[sorted.len() / 2] * 1.4826).max(1e-4);
        for (w, err) in weights.iter_mut().zip(errs.drain(..)) {
            let r = err / (1.5 * scale);
            *w = if r <= 1.0 { 1.0 } else { 1.0 / r };
        }
    }

    let tmp_fit = AlignFit {
        space: cfg.space,
        a: a as f32,
        b: b as f32,
        clip_above_m: f32::INFINITY,
        inlier_share: 0.0,
        median_rel_err: 0.0,
    };

    let mut rel_errs = Vec::with_capacity(pairs.len());
    let mut inlier_depths = Vec::new();
    for &(raw, gt) in &pairs {
        let pred = tmp_fit.apply_one(raw);
        let rel = (pred - gt).abs() / gt.max(1e-4);
        rel_errs.push(rel);
        if rel < 0.08 && pred.is_finite() {
            inlier_depths.push(pred);
        }
    }
    rel_errs.sort_by(f32::total_cmp);
    let median_rel_err = rel_errs[rel_errs.len() / 2];
    let inlier_share = inlier_depths.len() as f32 / pairs.len() as f32;
    let clip_above_m = if cfg.clip_to_support {
        inlier_depths.iter().copied().fold(0.0f32, f32::max).max(0.5)
    } else {
        f32::INFINITY
    };

    Some(AlignFit {
        space: cfg.space,
        a: a as f32,
        b: b as f32,
        clip_above_m,
        inlier_share,
        median_rel_err,
    })
}

pub fn ground_align(
    depth: &mut DepthMap,
    intr: &Intrinsics,
    pose: &Pose,
    height: Option<HeightFn<'_>>,
    raw_cap_m: Option<f32>,
    cfg: &AlignCfg,
) -> Option<AlignFit> {
    ground_align_with(depth, None, intr, pose, height, raw_cap_m, cfg)
}

pub fn ground_align_with(
    depth: &mut DepthMap,
    normals: Option<&NormalMap>,
    intr: &Intrinsics,
    pose: &Pose,
    height: Option<HeightFn<'_>>,
    raw_cap_m: Option<f32>,
    cfg: &AlignCfg,
) -> Option<AlignFit> {
    let fit = fit_ground_with(depth, normals, intr, pose, height, raw_cap_m, cfg)?;
    for d in &mut depth.data {
        *d = fit.apply_one(*d);
    }
    depth.kind = DepthKind::Metric;
    Some(fit)
}

pub fn contact_range(
    intr: &Intrinsics,
    pose: &Pose,
    mask: &Mask,
    height: Option<HeightFn<'_>>,
    min_range_m: f32,
    max_range_m: f32,
) -> Option<(f32, f32)> {
    let mut bottom_y: Option<u32> = None;
    let mut sx = 0.0f64;
    let mut n = 0u64;
    for y in 0..mask.height {
        for x in 0..mask.width {
            if mask.get(x, y) {
                bottom_y = Some(bottom_y.map_or(y, |b| b.max(y)));
            }
        }
    }
    let yb = bottom_y?;
    if yb + 1 >= mask.height {
        return None;
    }
    let py = (yb as f32 + 0.5) / mask.height.max(1) as f32 * intr.height as f32;
    for x in 0..mask.width {
        if mask.get(x, yb) {
            sx += (x as f32 + 0.5) as f64 / mask.width.max(1) as f64 * intr.width as f64;
            n += 1;
        }
    }
    if n == 0 {
        return None;
    }
    let px = (sx / n as f64) as f32;
    let d0 = ground_depth(intr, pose, px, py, height, max_range_m)?;
    if d0 < min_range_m {
        return None;
    }
    let range_m = intr.ground_range(pose.yaw, px, py, d0);
    let dy_px = 1.0f32;
    let d1 = ground_depth(intr, pose, px, (py - dy_px).max(0.5), height, max_range_m * 2.0)
        .unwrap_or(d0 * 1.05);
    let r1 = intr.ground_range(pose.yaw, px, (py - dy_px).max(0.5), d1);
    let sigma = (r1 - range_m).abs().max(0.02 * range_m);
    Some((range_m, sigma))
}
