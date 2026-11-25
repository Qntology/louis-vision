//! Empty-stage stereo Z-Keying, RGB motion NaN-hole recovery, `mask_ops` contour cleanup,
//! RGB Joint Bilateral upsampling (`AlphaMatte`), and unlimited 3D Kalman + Hungarian multi-performer tracking.

use std::collections::VecDeque;

use anyhow::{bail, Result};
use image::GrayImage;
use serde::{Deserialize, Serialize};

use crate::camera::{Intrinsics, Pose};
use crate::frame::{AlphaMatte, DepthMap, Mask, Rgb};
use crate::mask_ops::postprocess::{apply as postprocess_mask, PostprocessParams};
use crate::stereo::{stereo_sigma_z, StereoRig};

/// Static empty-stage background model built from 30+ calibration frames.
#[derive(Clone, Debug, PartialEq)]
pub struct StageBackground {
    pub width: u32,
    pub height: u32,
    pub z_bg: Vec<f32>,
    pub sigma_bg: Vec<f32>,
    pub rgb_bg: Option<Rgb>,
    pub frame_count: usize,
}

impl StageBackground {
    /// Captures per-pixel median depth and robust MAD noise floor (`sigma_bg`) across static calibration frames.
    pub fn capture_median(
        frames: &[&DepthMap],
        rgb_bg: Option<Rgb>,
        min_sigma_m: f32,
    ) -> Result<Self> {
        let Some(first) = frames.first() else {
            bail!("at least 1 background depth frame is required");
        };
        let (w, h) = (first.width, first.height);
        let n = (w * h) as usize;
        for f in frames {
            if f.width != w || f.height != h {
                bail!(
                    "background frame size mismatch: expected {}x{}, got {}x{}",
                    w,
                    h,
                    f.width,
                    f.height
                );
            }
        }
        if let Some(bg) = &rgb_bg {
            if bg.width == 0 || bg.height == 0 {
                bail!("empty rgb_bg frame");
            }
        }

        let mut z_bg = vec![f32::INFINITY; n];
        let mut sigma_bg = vec![min_sigma_m.max(0.005); n];
        let mut vals = Vec::with_capacity(frames.len());
        let mut devs = Vec::with_capacity(frames.len());

        for px in 0..n {
            vals.clear();
            for f in frames {
                let d = f.data[px];
                if d.is_finite() && d > 0.0 {
                    vals.push(d);
                }
            }
            if vals.is_empty() {
                continue;
            }
            vals.sort_by(f32::total_cmp);
            let med = vals[vals.len() / 2];
            z_bg[px] = med;
            devs.clear();
            for &v in &vals {
                devs.push((v - med).abs());
            }
            devs.sort_by(f32::total_cmp);
            let mad = devs[devs.len() / 2];
            sigma_bg[px] = (1.4826 * mad).max(min_sigma_m.max(0.005));
        }

        Ok(Self {
            width: w,
            height: h,
            z_bg,
            sigma_bg,
            rgb_bg,
            frame_count: frames.len(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZKeyCfg {
    pub tau_min_m: f32,
    pub k_sigma: f32,
    pub motion_luma_thresh: f32,
    pub bilateral_radius: i32,
    pub bilateral_sigma_spatial: f32,
    pub bilateral_sigma_range: f32,
    pub hole_fill: u32,
    pub dot_remove: u32,
    pub border_fix: u32,
    pub grow: i32,
    pub max_voxel_link_m: f32,
    pub min_cluster_px: usize,
    pub max_assoc_dist_m: f32,
    pub dt_s: f32,
    pub track_ttl_frames: u32,
}

impl Default for ZKeyCfg {
    fn default() -> Self {
        Self {
            tau_min_m: 0.15,
            k_sigma: 3.0,
            motion_luma_thresh: 24.0,
            bilateral_radius: 2,
            bilateral_sigma_spatial: 2.0,
            bilateral_sigma_range: 18.0,
            hole_fill: 4,
            dot_remove: 2,
            border_fix: 1,
            grow: 0,
            max_voxel_link_m: 0.45,
            min_cluster_px: 4,
            max_assoc_dist_m: 1.25,
            dt_s: 1.0 / 60.0,
            track_ttl_frames: 15,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PerformerTrack3D {
    pub id: u32,
    pub pos_m: [f32; 3],
    pub vel_mps: [f32; 3],
    pub bbox_px: [u32; 4],
    pub pixel_count: u32,
    pub missed_frames: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StageKeyOut {
    pub mask: Mask,
    pub matte: AlphaMatte,
    pub performers: Vec<PerformerTrack3D>,
    pub hole_recovered_px: u32,
}

#[derive(Clone, Debug)]
struct TrackState {
    id: u32,
    x: [f64; 6], // [px, py, pz, vx, vy, vz]
    missed: u32,
}

impl TrackState {
    fn new(id: u32, pos: [f32; 3]) -> Self {
        Self {
            id,
            x: [pos[0] as f64, pos[1] as f64, pos[2] as f64, 0.0, 0.0, 0.0],
            missed: 0,
        }
    }

    fn predict(&mut self, dt: f64) {
        self.x[0] += self.x[3] * dt;
        self.x[1] += self.x[4] * dt;
        self.x[2] += self.x[5] * dt;
    }

    fn update(&mut self, obs: [f32; 3], dt: f64) {
        let dt = dt.max(1e-3);
        let ix = obs[0] as f64 - self.x[0];
        let iy = obs[1] as f64 - self.x[1];
        let iz = obs[2] as f64 - self.x[2];
        let alpha = 0.65;
        let beta = 0.35 / dt;
        self.x[0] += alpha * ix;
        self.x[1] += alpha * iy;
        self.x[2] += alpha * iz;
        self.x[3] = 0.7 * self.x[3] + beta * ix;
        self.x[4] = 0.7 * self.x[4] + beta * iy;
        self.x[5] = 0.7 * self.x[5] + beta * iz;
        self.missed = 0;
    }

    fn pos_f32(&self) -> [f32; 3] {
        [self.x[0] as f32, self.x[1] as f32, self.x[2] as f32]
    }
}

/// Stateful 60fps empty-stage Z-Keyer + unlimited 3D Kalman-Hungarian multi-performer tracker.
pub struct StageZKeyTracker {
    pub cfg: ZKeyCfg,
    bg: StageBackground,
    tracks: Vec<TrackState>,
    next_id: u32,
    prev_rgb: Option<Rgb>,
}

impl StageZKeyTracker {
    pub fn new(bg: StageBackground, cfg: ZKeyCfg) -> Self {
        Self {
            cfg,
            bg,
            tracks: Vec::new(),
            next_id: 1,
            prev_rgb: None,
        }
    }

    pub fn active_tracks(&self) -> usize {
        self.tracks.len()
    }

    pub fn step(
        &mut self,
        rgb: &Rgb,
        depth: &DepthMap,
        intr: &Intrinsics,
        pose: &Pose,
        rig: Option<StereoRig>,
    ) -> Result<StageKeyOut> {
        let (dw, dh) = (self.bg.width as usize, self.bg.height as usize);
        if depth.width != self.bg.width || depth.height != self.bg.height {
            bail!(
                "depth grid {}x{} does not match StageBackground {}x{}",
                depth.width,
                depth.height,
                self.bg.width,
                self.bg.height
            );
        }

        let mut fg_raw = vec![0u8; dw * dh];
        let mut hole_recovered_px = 0u32;
        let ref_rgb = self.bg.rgb_bg.as_ref().or(self.prev_rgb.as_ref());

        let f_grid = rig.map(|r| r.focal_on(rgb.width, depth.width)).unwrap_or(0.0);

        for y in 0..dh {
            for x in 0..dw {
                let i = y * dw + x;
                let z_cur = depth.data[i];
                let z_bg = self.bg.z_bg[i];
                let s_bg = self.bg.sigma_bg[i];

                if z_cur.is_finite() && z_cur > 0.0 {
                    let s_z = rig
                        .map(|r| stereo_sigma_z(&r, f_grid, z_cur))
                        .filter(|v| v.is_finite())
                        .unwrap_or(0.02 * z_cur);
                    let tau = self
                        .cfg
                        .tau_min_m
                        .max(self.cfg.k_sigma * (s_bg * s_bg + s_z * s_z).sqrt());
                    if z_bg - z_cur > tau {
                        fg_raw[i] = 255;
                    }
                } else if let Some(ref_im) = ref_rgb {
                    // Textureless / specular NaN disparity recovery from RGB temporal/bg motion delta.
                    let rx = ((x as f32 + 0.5) / dw as f32 * rgb.width as f32)
                        .clamp(0.0, rgb.width as f32 - 1.0) as u32;
                    let ry = ((y as f32 + 0.5) / dh as f32 * rgb.height as f32)
                        .clamp(0.0, rgb.height as f32 - 1.0) as u32;
                    let c = rgb.px(rx, ry);
                    let brx = ((x as f32 + 0.5) / dw as f32 * ref_im.width as f32)
                        .clamp(0.0, ref_im.width as f32 - 1.0) as u32;
                    let bry = ((y as f32 + 0.5) / dh as f32 * ref_im.height as f32)
                        .clamp(0.0, ref_im.height as f32 - 1.0) as u32;
                    let b = ref_im.px(brx, bry);
                    let dl = (0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32)
                        - (0.299 * b[0] as f32 + 0.587 * b[1] as f32 + 0.114 * b[2] as f32);
                    let chroma_d = ((c[0] as i32 - b[0] as i32).abs()
                        + (c[1] as i32 - b[1] as i32).abs()
                        + (c[2] as i32 - b[2] as i32).abs()) as f32
                        / 3.0;
                    if dl.abs() > self.cfg.motion_luma_thresh || chroma_d > self.cfg.motion_luma_thresh {
                        fg_raw[i] = 255;
                        hole_recovered_px += 1;
                    }
                }
            }
        }

        let coarse_gray = GrayImage::from_raw(dw as u32, dh as u32, fg_raw)
            .expect("depth foreground buffer size matches");
        let border_fix = if dw > 2 * self.cfg.border_fix as usize && dh > 2 * self.cfg.border_fix as usize {
            self.cfg.border_fix
        } else {
            0
        };
        let params = PostprocessParams {
            holes: self.cfg.hole_fill,
            dots: self.cfg.dot_remove,
            border_fix,
            grow: self.cfg.grow,
        };
        let cleaned = postprocess_mask(&coarse_gray, &params)?;

        let mask = Mask::from_gray(&cleaned);
        let matte = joint_bilateral_upsample(
            rgb,
            cleaned.as_raw(),
            dw as u32,
            dh as u32,
            self.cfg.bilateral_radius,
            self.cfg.bilateral_sigma_spatial,
            self.cfg.bilateral_sigma_range,
        );

        let clusters = cluster_3d_foreground(&mask, depth, intr, pose, self.cfg.max_voxel_link_m, self.cfg.min_cluster_px);
        let performers = self.associate_and_update(&clusters);
        self.prev_rgb = Some(rgb.clone());

        Ok(StageKeyOut {
            mask,
            matte,
            performers,
            hole_recovered_px,
        })
    }

    fn associate_and_update(&mut self, clusters: &[Cluster3D]) -> Vec<PerformerTrack3D> {
        let dt = self.cfg.dt_s as f64;
        for t in &mut self.tracks {
            t.predict(dt);
        }

        let n_tracks = self.tracks.len();
        let n_obs = clusters.len();
        let assignments = hungarian_assign(
            n_tracks,
            n_obs,
            self.cfg.max_assoc_dist_m as f64,
            |ti, oi| {
                let p = self.tracks[ti].pos_f32();
                let q = clusters[oi].centroid;
                let d2 = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2);
                d2.sqrt() as f64
            },
        );

        let mut matched_track = vec![false; n_tracks];
        let mut matched_obs = vec![false; n_obs];
        let mut out = Vec::new();

        for (ti, oi) in assignments {
            matched_track[ti] = true;
            matched_obs[oi] = true;
            self.tracks[ti].update(clusters[oi].centroid, dt);
            let tr = &self.tracks[ti];
            out.push(PerformerTrack3D {
                id: tr.id,
                pos_m: tr.pos_f32(),
                vel_mps: [tr.x[3] as f32, tr.x[4] as f32, tr.x[5] as f32],
                bbox_px: clusters[oi].bbox,
                pixel_count: clusters[oi].count,
                missed_frames: 0,
            });
        }

        for (oi, cl) in clusters.iter().enumerate() {
            if !matched_obs[oi] {
                let id = self.next_id;
                self.next_id += 1;
                let tr = TrackState::new(id, cl.centroid);
                out.push(PerformerTrack3D {
                    id,
                    pos_m: tr.pos_f32(),
                    vel_mps: [0.0; 3],
                    bbox_px: cl.bbox,
                    pixel_count: cl.count,
                    missed_frames: 0,
                });
                self.tracks.push(tr);
            }
        }

        let mut keep = Vec::with_capacity(self.tracks.len());
        for (ti, mut tr) in self.tracks.drain(..).enumerate() {
            if ti < matched_track.len() && !matched_track[ti] {
                tr.missed += 1;
            }
            if tr.missed <= self.cfg.track_ttl_frames {
                keep.push(tr);
            }
        }
        self.tracks = keep;
        out.sort_by_key(|p| p.id);
        out
    }
}

pub fn joint_bilateral_upsample(
    rgb: &Rgb,
    coarse_alpha: &[u8],
    cw: u32,
    ch: u32,
    radius: i32,
    sigma_s: f32,
    sigma_r: f32,
) -> AlphaMatte {
    let (w, h) = (rgb.width, rgb.height);
    let mut alpha = vec![0u8; (w * h) as usize];
    let r = radius.max(1);
    let inv_s2 = 0.5 / (sigma_s.max(0.5) * sigma_s.max(0.5));
    let inv_r2 = 0.5 / (sigma_r.max(1.0) * sigma_r.max(1.0));
    let scale_x = cw as f32 / w.max(1) as f32;
    let scale_y = ch as f32 / h.max(1) as f32;

    for y in 0..h {
        let cy_f = (y as f32 + 0.5) * scale_y - 0.5;
        let cy0 = cy_f.round() as i32;
        for x in 0..w {
            let cx_f = (x as f32 + 0.5) * scale_x - 0.5;
            let cx0 = cx_f.round() as i32;
            let p0 = rgb.px(x, y);
            let mut sum_w = 0.0f32;
            let mut sum_a = 0.0f32;

            for dy in -r..=r {
                let ny = cy0 + dy;
                if ny < 0 || ny >= ch as i32 {
                    continue;
                }
                let ry = (((ny as f32 + 0.5) / scale_y.max(1e-6)) as i32).clamp(0, h as i32 - 1) as u32;
                for dx in -r..=r {
                    let nx = cx0 + dx;
                    if nx < 0 || nx >= cw as i32 {
                        continue;
                    }
                    let rx = (((nx as f32 + 0.5) / scale_x.max(1e-6)) as i32).clamp(0, w as i32 - 1) as u32;
                    let pn = rgb.px(rx, ry);
                    let d2 = (dx * dx + dy * dy) as f32;
                    let dc = (p0[0] as f32 - pn[0] as f32).powi(2)
                        + (p0[1] as f32 - pn[1] as f32).powi(2)
                        + (p0[2] as f32 - pn[2] as f32).powi(2);
                    let w_ij = (-d2 * inv_s2 - dc * inv_r2).exp();
                    let a_val = coarse_alpha[(ny as u32 * cw + nx as u32) as usize] as f32;
                    sum_w += w_ij;
                    sum_a += w_ij * a_val;
                }
            }
            let out_a = if sum_w > 1e-8 { sum_a / sum_w } else { 0.0 };
            alpha[(y * w + x) as usize] = out_a.clamp(0.0, 255.0).round() as u8;
        }
    }
    AlphaMatte { width: w, height: h, alpha }
}

#[derive(Clone, Debug)]
struct Cluster3D {
    centroid: [f32; 3],
    bbox: [u32; 4],
    count: u32,
}

fn cluster_3d_foreground(
    mask: &Mask,
    depth: &DepthMap,
    intr: &Intrinsics,
    pose: &Pose,
    max_link_m: f32,
    min_px: usize,
) -> Vec<Cluster3D> {
    let (w, h) = (mask.width as usize, mask.height as usize);
    let sx = intr.width as f32 / mask.width.max(1) as f32;
    let sy = intr.height as f32 / mask.height.max(1) as f32;
    let max_d2 = max_link_m * max_link_m;

    let mut pts3d: Vec<Option<[f32; 3]>> = vec![None; w * h];
    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            if !mask.get(x as u32, y as u32) {
                continue;
            }
            let d = depth.at_scaled(x as f32 + 0.5, y as f32 + 0.5, mask.width, mask.height);
            if d.is_finite() && d > 0.0 && d < 50.0 {
                let r = intr.ray(pose.yaw, (x as f32 + 0.5) * sx, (y as f32 + 0.5) * sy);
                pts3d[idx] = Some([pose.x + d * r[0], pose.eye_m + d * r[1], pose.z + d * r[2]]);
            }
        }
    }

    let mut visited = vec![false; w * h];
    let mut queue = VecDeque::new();
    let mut out = Vec::new();

    for start in 0..w * h {
        if visited[start] || !mask.bits[start] {
            continue;
        }
        visited[start] = true;
        queue.push_back(start);
        let mut sx_m = 0.0f64;
        let mut sy_m = 0.0f64;
        let mut sz_m = 0.0f64;
        let mut n_3d = 0u64;
        let mut count = 0u32;
        let (x0, y0) = ((start % w) as u32, (start / w) as u32);
        let mut bbox = [x0, y0, x0, y0];

        while let Some(idx) = queue.pop_front() {
            count += 1;
            let (cx, cy) = ((idx % w) as u32, (idx / w) as u32);
            bbox[0] = bbox[0].min(cx);
            bbox[1] = bbox[1].min(cy);
            bbox[2] = bbox[2].max(cx);
            bbox[3] = bbox[3].max(cy);
            if let Some(p) = pts3d[idx] {
                sx_m += p[0] as f64;
                sy_m += p[1] as f64;
                sz_m += p[2] as f64;
                n_3d += 1;
            }
            let x = idx % w;
            let y = idx / w;
            for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    let ni = ny * w + nx;
                    if visited[ni] || !mask.bits[ni] {
                        continue;
                    }
                    // Prevent cross-depth bleeding between identically clothed crossing performers.
                    let depth_ok = match (pts3d[idx], pts3d[ni]) {
                        (Some(a), Some(b)) => {
                            let d2 = (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2);
                            d2 <= max_d2
                        }
                        _ => true,
                    };
                    if depth_ok {
                        visited[ni] = true;
                        queue.push_back(ni);
                    }
                }
            }
        }

        if count as usize >= min_px && n_3d > 0 {
            out.push(Cluster3D {
                centroid: [(sx_m / n_3d as f64) as f32, (sy_m / n_3d as f64) as f32, (sz_m / n_3d as f64) as f32],
                bbox,
                count,
            });
        }
    }
    out
}

/// Exact min-cost Hungarian bipartite assignment gated by `max_cost`.
fn hungarian_assign(
    n_rows: usize,
    n_cols: usize,
    max_cost: f64,
    cost_fn: impl Fn(usize, usize) -> f64,
) -> Vec<(usize, usize)> {
    if n_rows == 0 || n_cols == 0 {
        return Vec::new();
    }
    let dim = n_rows.max(n_cols);
    let gate = max_cost + 1.0;
    let mut a = vec![vec![gate; dim + 1]; dim + 1];
    for i in 1..=n_rows {
        for j in 1..=n_cols {
            let c = cost_fn(i - 1, j - 1);
            if c <= max_cost {
                a[i][j] = c;
            }
        }
    }

    let mut u = vec![0.0f64; dim + 1];
    let mut v = vec![0.0f64; dim + 1];
    let mut p = vec![0usize; dim + 1];
    let mut way = vec![0usize; dim + 1];

    for i in 1..=dim {
        p[0] = i;
        let mut j0 = 0usize;
        let mut minv = vec![f64::INFINITY; dim + 1];
        let mut used = vec![false; dim + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let mut delta = f64::INFINITY;
            let mut j1 = 0usize;
            for j in 1..=dim {
                if !used[j] {
                    let cur = a[i0][j] - u[i0] - v[j];
                    if cur < minv[j] {
                        minv[j] = cur;
                        way[j] = j0;
                    }
                    if minv[j] < delta {
                        delta = minv[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=dim {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }

    let mut matches = Vec::new();
    for j in 1..=n_cols {
        let i = p[j];
        if i >= 1 && i <= n_rows && a[i][j] <= max_cost {
            matches.push((i - 1, j - 1));
        }
    }
    matches
}
