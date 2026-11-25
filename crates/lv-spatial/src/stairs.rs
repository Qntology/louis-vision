//! Scale-invariant forward corridor step/curb detection along camera optical axis.

use serde::{Deserialize, Serialize};

use crate::camera::{Intrinsics, Pose};
use crate::frame::DepthMap;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepCfg {
    pub near_m: f32,
    pub far_m: f32,
    pub half_width_m: f32,
    pub bin_m: f32,
    pub min_step_m: f32,
}

impl Default for StepCfg {
    fn default() -> Self {
        Self {
            near_m: 0.35,
            far_m: 3.0,
            half_width_m: 0.45,
            bin_m: 0.08,
            min_step_m: 0.045,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepRead {
    pub dist_m: f32,
    pub height_m: f32,
    pub coverage: f32,
}

pub fn read_step(
    depth: &DepthMap,
    intr: &Intrinsics,
    pose: &Pose,
    cfg: &StepCfg,
) -> Option<StepRead> {
    let n = ((cfg.far_m - cfg.near_m) / cfg.bin_m).ceil().max(4.0) as usize;
    let mut bins: Vec<Vec<(f32, f32)>> = vec![Vec::new(); n];
    let sx = intr.width as f32 / depth.width.max(1) as f32;
    let sy = intr.height as f32 / depth.height.max(1) as f32;
    let (cy, sy_yaw) = (pose.yaw.cos(), pose.yaw.sin());

    let mut samples = Vec::new();
    for y in 0..depth.height {
        for x in 0..depth.width {
            let d = depth.at(x, y);
            if !d.is_finite() || d <= 0.0 {
                continue;
            }
            let px = (x as f32 + 0.5) * sx;
            let py = (y as f32 + 0.5) * sy;
            let r = intr.ray(pose.yaw, px, py);
            let fwd = (r[0] * cy + r[2] * sy_yaw) * d;
            let lat = (-r[0] * sy_yaw + r[2] * cy) * d;
            if lat.abs() <= cfg.half_width_m && fwd >= 0.25 && fwd <= cfg.far_m * 1.4 {
                samples.push((fwd, pose.eye_m + r[1] * d));
            }
        }
    }
    if samples.len() < 16 {
        return None;
    }

    // Near-band scale alignment onto eye height so uncalibrated scale errors cancel out.
    let mut near_heights: Vec<f32> = samples
        .iter()
        .filter(|(f, _)| *f <= cfg.near_m + 0.45)
        .map(|(_, _)| 1.0f32)
        .collect();
    near_heights.clear();
    let mut scale_ratios: Vec<f32> = samples
        .iter()
        .filter(|(f, _)| *f >= cfg.near_m && *f <= cfg.near_m + 0.45)
        .filter_map(|(_, h)| {
            let off = pose.eye_m - h;
            (off > 0.2).then_some(pose.eye_m / off)
        })
        .collect();
    let scale = if scale_ratios.len() >= 4 {
        scale_ratios.sort_by(f32::total_cmp);
        scale_ratios[scale_ratios.len() / 2].clamp(0.4, 2.5)
    } else {
        1.0
    };

    for (f_raw, h_raw) in samples {
        let f = f_raw * scale;
        let h = pose.eye_m - (pose.eye_m - h_raw) * scale;
        if f >= cfg.near_m && f < cfg.far_m {
            let idx = ((f - cfg.near_m) / cfg.bin_m) as usize;
            if idx < n {
                bins[idx].push((f, h));
            }
        }
    }

    let mut med: Vec<Option<(f32, f32)>> = Vec::with_capacity(n);
    let mut valid_bins = 0usize;
    for mut b in bins {
        if b.len() < 3 {
            med.push(None);
            continue;
        }
        valid_bins += 1;
        b.sort_by(|a, c| a.1.total_cmp(&c.1));
        med.push(Some(b[b.len() / 2]));
    }
    let coverage = valid_bins as f32 / n as f32;
    if coverage < 0.35 || valid_bins < 4 {
        return None;
    }

    let base = med.iter().flatten().take(3).map(|x| x.1).sum::<f32>()
        / (med.iter().flatten().take(3).count().max(1) as f32);

    let mut best: Option<(usize, f32)> = None;
    for (idx, item) in med.iter().enumerate() {
        let Some((_, h)) = item else { continue };
        let dh = h - base;
        if dh.abs() >= cfg.min_step_m {
            if best.is_none_or(|(_, bdh)| dh.abs() > bdh.abs()) {
                best = Some((idx, dh));
            }
        }
    }

    let (idx, _) = best?;
    let mut plateau: Vec<f32> = med[idx..]
        .iter()
        .flatten()
        .map(|x| x.1 - base)
        .filter(|d| d.abs() >= cfg.min_step_m * 0.6)
        .collect();
    if plateau.is_empty() {
        return None;
    }
    plateau.sort_by(f32::total_cmp);
    let height_m = plateau[plateau.len() / 2];
    let dist_m = cfg.near_m + (idx as f32 + 0.5) * cfg.bin_m;
    Some(StepRead { dist_m, height_m, coverage })
}
