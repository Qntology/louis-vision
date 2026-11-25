//! Patent-clear single-volume 4/8-path Census-SGBM stereo matcher delegating inner Census &
//! path aggregation loops to `lv_kernels::census` (0 B ML weights; honest `NaN` / `INFINITY`).

use std::time::Instant;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use lv_kernels::census::{census_5x5_u32, census_cost_volume, sgbm_aggregate};

use crate::depth::DepthModel;
use crate::frame::{DepthKind, DepthMap, Rgb};
use crate::pyramid::{Pyramid, PyramidCfg};

pub const MODE: &str = "sgbm-stereo";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StereoRig {
    pub focal_px: f32,
    pub baseline_m: f32,
    pub subpixel_px: f32,
}

impl StereoRig {
    pub fn focal_on(&self, frame_w: u32, grid_w: u32) -> f32 {
        self.focal_px * grid_w.max(1) as f32 / frame_w.max(1) as f32
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubpixelFit {
    Parabola,
    #[default]
    Equiangular,
}

fn default_max_rel_sigma() -> f32 {
    0.25
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SgbmCfg {
    pub per_l2: u32,
    pub min_depth_m: f32,
    pub max_depth_m: f32,
    pub block: u32,
    pub paths: u8,
    pub p1: u16,
    pub p2: u16,
    #[serde(default)]
    pub p2_edge: f32,
    pub uniqueness: f32,
    pub lr_max_diff: f32,
    pub min_texture: f32,
    pub saturated: u8,
    pub speckle_size: usize,
    pub speckle_range: f32,
    #[serde(default)]
    pub subpixel: SubpixelFit,
    #[serde(default = "default_max_rel_sigma")]
    pub max_rel_sigma: f32,
}

impl Default for SgbmCfg {
    fn default() -> Self {
        Self {
            per_l2: 8,
            min_depth_m: 0.25,
            max_depth_m: 12.0,
            block: 1,
            paths: 8,
            p1: 10,
            p2: 120,
            p2_edge: 12.0,
            uniqueness: 0.10,
            lr_max_diff: 1.0,
            min_texture: 2.0,
            saturated: 250,
            speckle_size: 24,
            speckle_range: 1.0,
            subpixel: SubpixelFit::Equiangular,
            max_rel_sigma: default_max_rel_sigma(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StereoStats {
    pub width: u32,
    pub height: u32,
    pub disparities: u32,
    pub focal_px: f32,
    pub baseline_m: f32,
    pub valid: u32,
    pub far: u32,
    pub border: u32,
    pub saturated: u32,
    pub no_texture: u32,
    pub non_unique: u32,
    pub occluded: u32,
    pub zero_disp: u32,
    pub speckle: u32,
    pub elapsed_ms: f32,
}

impl StereoStats {
    pub fn pixels(&self) -> u32 {
        self.width * self.height
    }

    pub fn valid_frac(&self) -> f32 {
        self.valid as f32 / self.pixels().max(1) as f32
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StereoOut {
    pub depth: DepthMap,
    pub disparity: Vec<f32>,
    pub stats: StereoStats,
}

pub fn stereo_grid(width: u32, height: u32, pyramid: Option<&PyramidCfg>, per_l2: u32) -> (u32, u32) {
    let cfg = pyramid.copied().unwrap_or_default();
    let (l2w, l2h) = Pyramid::for_frame(width, height, &cfg).l2();
    let mut k = per_l2.max(1).next_power_of_two();
    if k > per_l2.max(1) {
        k /= 2;
    }
    while k > 1 && (l2w * k > width.max(1) || l2h * k > height.max(1)) {
        k /= 2;
    }
    (l2w * k, l2h * k)
}

#[inline]
pub fn stereo_sigma_z(rig: &StereoRig, focal_grid_px: f32, depth_m: f32) -> f32 {
    if !depth_m.is_finite() || depth_m <= 0.0 || focal_grid_px <= 0.0 || rig.baseline_m <= 0.0 {
        return f32::NAN;
    }
    depth_m * depth_m * rig.subpixel_px / (focal_grid_px * rig.baseline_m)
}

fn area_weights(src: u32, dst: u32) -> Vec<Vec<(u32, f32)>> {
    let s = src as f32 / dst.max(1) as f32;
    (0..dst)
        .map(|i| {
            let (a, b) = (i as f32 * s, (i + 1) as f32 * s);
            let mut v = Vec::new();
            let mut j = a.floor() as u32;
            while (j as f32) < b && j < src {
                let w = ((j + 1) as f32).min(b) - (j as f32).max(a);
                if w > 1e-6 {
                    v.push((j, w / s));
                }
                j += 1;
            }
            if v.is_empty() {
                v.push(((a as u32).min(src - 1), 1.0));
            }
            v
        })
        .collect()
}

fn grey_on(rgb: &Rgb, gw: u32, gh: u32) -> (Vec<f32>, Vec<u8>) {
    let (w, h) = (rgb.width, rgb.height);
    let luma = |x: u32, y: u32| {
        let p = rgb.px(x, y);
        (
            0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32,
            p[0].max(p[1]).max(p[2]),
        )
    };
    if (gw, gh) == (w, h) {
        let mut g = Vec::with_capacity((w * h) as usize);
        let mut m = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let (l, b) = luma(x, y);
                g.push(l);
                m.push(b);
            }
        }
        return (g, m);
    }
    let (wx, wy) = (area_weights(w, gw), area_weights(h, gh));
    let mut g = vec![0.0f32; (gw * gh) as usize];
    let mut m = vec![0u8; (gw * gh) as usize];
    for (gy, ys) in wy.iter().enumerate() {
        for (gx, xs) in wx.iter().enumerate() {
            let (mut acc, mut hi) = (0.0f32, 0u8);
            for &(sy, ky) in ys {
                for &(sx, kx) in xs {
                    let (l, b) = luma(sx, sy);
                    acc += l * kx * ky;
                    hi = hi.max(b);
                }
            }
            g[gy * gw as usize + gx] = acc;
            m[gy * gw as usize + gx] = hi;
        }
    }
    (g, m)
}

fn subpixel_delta(cm: f32, c0: f32, cp: f32, mode: SubpixelFit) -> f32 {
    match mode {
        SubpixelFit::Parabola => {
            let denom = 2.0 * (cm + cp - 2.0 * c0);
            if denom.abs() <= 1e-6 { 0.0 } else { ((cm - cp) / denom).clamp(-0.5, 0.5) }
        }
        SubpixelFit::Equiangular => {
            let max_s = cm.max(cp);
            let denom = 2.0 * (max_s - c0);
            if denom <= 1e-6 { 0.0 } else { ((cm - cp) / denom).clamp(-0.5, 0.5) }
        }
    }
}

fn remove_speckles(disp: &mut [f32], w: usize, h: usize, max_size: usize, range: f32) -> u32 {
    if max_size == 0 || range <= 0.0 {
        return 0;
    }
    let mut visited = vec![false; w * h];
    let mut stack = Vec::new();
    let mut comp = Vec::new();
    let mut dropped = 0u32;

    for start in 0..w * h {
        if visited[start] || !disp[start].is_finite() {
            continue;
        }
        visited[start] = true;
        stack.push(start);
        comp.clear();
        while let Some(idx) = stack.pop() {
            comp.push(idx);
            let d0 = disp[idx];
            let (x, y) = (idx % w, idx / w);
            for (nx, ny) in [
                (x.wrapping_sub(1), y),
                (x + 1, y),
                (x, y.wrapping_sub(1)),
                (x, y + 1),
            ] {
                if nx < w && ny < h {
                    let ni = ny * w + nx;
                    if !visited[ni] && disp[ni].is_finite() && (disp[ni] - d0).abs() <= range {
                        visited[ni] = true;
                        stack.push(ni);
                    }
                }
            }
        }
        if comp.len() < max_size {
            dropped += comp.len() as u32;
            for &idx in &comp {
                disp[idx] = f32::NAN;
            }
        }
    }
    dropped
}

pub fn sgbm(
    left: &Rgb,
    right: &Rgb,
    rig: &StereoRig,
    cfg: &SgbmCfg,
    pyramid: Option<&PyramidCfg>,
) -> Result<StereoOut> {
    if (left.width, left.height) != (right.width, right.height) {
        bail!(
            "stereo pair size mismatch: left {}x{}, right {}x{}",
            left.width,
            left.height,
            right.width,
            right.height
        );
    }
    if rig.baseline_m <= 0.0 || rig.focal_px <= 0.0 {
        bail!("invalid stereo rig: focal={} baseline={}", rig.focal_px, rig.baseline_m);
    }

    let t0 = Instant::now();
    let (gw, gh) = stereo_grid(left.width, left.height, pyramid, cfg.per_l2);
    let (w, h) = (gw as usize, gh as usize);
    let f = rig.focal_on(left.width, gw);
    let fb = f * rig.baseline_m;
    let nd = ((fb / cfg.min_depth_m.max(0.05)).ceil() as usize + 2)
        .next_multiple_of(8)
        .clamp(8, (w / 2).max(8));

    let ((gl, ml), (gr, mr)) = rayon::join(|| grey_on(left, gw, gh), || grey_on(right, gw, gh));

    let ((cl, sd_l), (cr, _)) = rayon::join(|| census_5x5_u32(&gl, w, h), || census_5x5_u32(&gr, w, h));
    let cost = census_cost_volume(&cl, &cr, w, h, nd, cfg.block as usize);
    let agg = sgbm_aggregate(&cost, &gl, w, h, nd, cfg.paths, cfg.p1, cfg.p2, cfg.p2_edge);

    // Precompute right-eye best disparities diagonally from the single aggregated left cost volume:
    // S_R(xr, d) = S(xr + d, d).
    use rayon::prelude::*;
    let mut right_disp = vec![-1i32; w * h];
    right_disp.par_chunks_mut(w).enumerate().for_each(|(y, r_row)| {
        for xr in 0..w {
            let (mut best, mut bd) = (u16::MAX, -1i32);
            for d in 0..nd.min(w - xr) {
                let c = agg[(y * w + xr + d) * nd + d];
                if c < best {
                    best = c;
                    bd = d as i32;
                }
            }
            r_row[xr] = bd;
        }
    });

    let margin = 2 + cfg.block as usize;
    let mut disp = vec![f32::NAN; w * h];
    let mut stats = StereoStats {
        width: gw,
        height: gh,
        disparities: nd as u32,
        focal_px: f,
        baseline_m: rig.baseline_m,
        ..StereoStats::default()
    };

    for y in 0..h {
        let border_y = y < margin || y + margin >= h;
        for x in 0..w {
            let p = y * w + x;
            if border_y || x < margin + 1 || x + margin >= w {
                stats.border += 1;
                continue;
            }
            if ml[p] >= cfg.saturated {
                stats.saturated += 1;
                continue;
            }
            if sd_l[p] < cfg.min_texture {
                stats.no_texture += 1;
                continue;
            }
            let row = &agg[p * nd..(p + 1) * nd];
            let top = nd.min(x + 1);
            let mut best_d = 0usize;
            let mut best_c = u16::MAX;
            for (d, &v) in row.iter().enumerate().take(top) {
                if v < best_c {
                    best_c = v;
                    best_d = d;
                }
            }
            if best_d == 0 {
                stats.zero_disp += 1;
                continue;
            }
            let second_c = row
                .iter()
                .enumerate()
                .take(top)
                .filter(|(d, _)| d.abs_diff(best_d) > 1)
                .map(|(_, &v)| v)
                .min()
                .unwrap_or(u16::MAX);
            if (second_c as f32) * (1.0 - cfg.uniqueness) <= best_c as f32 {
                stats.non_unique += 1;
                continue;
            }
            let raw_row = &cost[p * nd..p * nd + top];
            if raw_row
                .iter()
                .enumerate()
                .any(|(d, &v)| d.abs_diff(best_d) > 1 && v <= raw_row[best_d])
            {
                stats.non_unique += 1;
                continue;
            }

            let delta = if best_d > 0 && best_d + 1 < top {
                subpixel_delta(
                    row[best_d - 1] as f32,
                    best_c as f32,
                    row[best_d + 1] as f32,
                    cfg.subpixel,
                )
            } else {
                0.0
            };
            let d_sub = best_d as f32 + delta;
            if d_sub <= 0.1 {
                stats.zero_disp += 1;
                continue;
            }

            let xr = x as i64 - d_sub.round() as i64;
            let lr_ok = xr >= 0
                && right_disp[y * w + xr as usize] >= 0
                && (d_sub - right_disp[y * w + xr as usize] as f32).abs() <= cfg.lr_max_diff;
            if !lr_ok {
                stats.occluded += 1;
                continue;
            }
            if mr[y * w + xr as usize] >= cfg.saturated {
                stats.saturated += 1;
                continue;
            }

            disp[p] = d_sub;
        }
    }

    stats.speckle = remove_speckles(&mut disp, w, h, cfg.speckle_size, cfg.speckle_range);

    let z_far = cfg
        .max_depth_m
        .min(cfg.max_rel_sigma.max(1e-3) * fb / rig.subpixel_px.max(0.01));
    let mut depth_data = vec![f32::NAN; w * h];
    for (p, &d) in disp.iter().enumerate() {
        if !d.is_finite() {
            continue;
        }
        let z = fb / d;
        if z > z_far {
            depth_data[p] = f32::INFINITY;
            stats.far += 1;
        } else {
            depth_data[p] = z;
            stats.valid += 1;
        }
    }
    stats.elapsed_ms = t0.elapsed().as_secs_f32() * 1000.0;

    Ok(StereoOut {
        depth: DepthMap {
            width: gw,
            height: gh,
            kind: DepthKind::Metric,
            data: depth_data,
        },
        disparity: disp,
        stats,
    })
}

pub struct SgbmDepth {
    pub rig: StereoRig,
    pub cfg: SgbmCfg,
    pyramid: Option<PyramidCfg>,
    last: Option<StereoStats>,
}

impl SgbmDepth {
    pub fn new(rig: StereoRig, cfg: SgbmCfg) -> Self {
        Self {
            rig,
            cfg,
            pyramid: Some(PyramidCfg::default()),
            last: None,
        }
    }

    pub fn grid_for(&self, width: u32, height: u32) -> (u32, u32) {
        stereo_grid(width, height, self.pyramid.as_ref(), self.cfg.per_l2)
    }
}

impl DepthModel for SgbmDepth {
    fn mode(&self) -> &str {
        MODE
    }

    fn max_depth_m(&self) -> Option<f32> {
        Some(self.cfg.max_depth_m)
    }

    fn infer(&mut self, _rgb: &Rgb) -> Result<DepthMap> {
        bail!("{} requires a stereo pair via infer_pair(left, right)", MODE)
    }

    fn set_pyramid(&mut self, cfg: Option<&PyramidCfg>) {
        self.pyramid = cfg.copied();
    }

    fn stereo_rig(&self) -> Option<StereoRig> {
        Some(self.rig)
    }

    fn infer_pair(&mut self, left: &Rgb, right: &Rgb) -> Result<DepthMap> {
        let out = sgbm(left, right, &self.rig, &self.cfg, self.pyramid.as_ref())?;
        self.last = Some(out.stats);
        Ok(out.depth)
    }

    fn stereo_stats(&self) -> Option<StereoStats> {
        self.last.clone()
    }
}
