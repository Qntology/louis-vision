//! 1:2:4 multi-rate vision pyramid (`L1 <= 256` tokens, `L2 = 2*L1`, `mask = 16*L2`),
//! sky/far patch gating, and depth-discontinuity semantic point snapping.

use serde::{Deserialize, Serialize};

use crate::align::ground_depth;
use crate::camera::{Intrinsics, Pose};
use crate::frame::{DepthMap, SegMap};
use crate::locate::SemanticPoint;

pub const L1_BUDGET: usize = 256;
pub const L2_RATIO: usize = 2;
pub const MASK_PER_CELL: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellGateCfg {
    pub enabled: bool,
    pub sky_max_frac: f32,
    pub min_near_frac: f32,
    pub far_m: f32,
}

impl Default for CellGateCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            sky_max_frac: 0.5,
            min_near_frac: 0.25,
            far_m: 18.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapCfg {
    pub enabled: bool,
    pub edge_frac: f32,
    pub ground_slack_m: f32,
}

impl Default for SnapCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            edge_frac: 0.20,
            ground_slack_m: 0.25,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PyramidCfg {
    pub l1_short: usize,
    pub l1_budget: usize,
    pub gate: CellGateCfg,
    pub snap: SnapCfg,
}

impl Default for PyramidCfg {
    fn default() -> Self {
        Self {
            l1_short: 12,
            l1_budget: L1_BUDGET,
            gate: CellGateCfg::default(),
            snap: SnapCfg::default(),
        }
    }
}

impl PyramidCfg {
    pub fn fast() -> Self {
        Self {
            l1_short: 12,
            l1_budget: L1_BUDGET,
            gate: CellGateCfg::default(),
            snap: SnapCfg { enabled: false, ..SnapCfg::default() },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pyramid {
    pub width: u32,
    pub height: u32,
    pub cols: u32,
    pub rows: u32,
    pub subs: u32,
}

/// Computes `(cols, rows)` for L1 pyramid grid matching `l1_short` and `l1_budget`.
pub fn pyramid_compat_grid(width: u32, height: u32, l1_short: usize, l1_budget: usize) -> (usize, usize) {
    let p = Pyramid::for_frame(
        width,
        height,
        &PyramidCfg { l1_short, l1_budget, ..PyramidCfg::default() },
    );
    (p.cols as usize, p.rows as usize)
}

impl Pyramid {
    pub fn for_frame(width: u32, height: u32, cfg: &PyramidCfg) -> Self {
        let (w, h) = (width.max(1), height.max(1));
        let budget = cfg.l1_budget.max(1);
        let short = cfg.l1_short.max(1);
        let (cols, rows) = if w >= h {
            let r = short.min(budget);
            let c = ((r as f64 * w as f64 / h as f64).round() as usize).max(1);
            let scale = if c * r > budget {
                (budget as f64 / (c * r) as f64).sqrt()
            } else {
                1.0
            };
            (
                ((c as f64 * scale).floor() as usize).max(1) as u32,
                ((r as f64 * scale).floor() as usize).max(1) as u32,
            )
        } else {
            let c = short.min(budget);
            let r = ((c as f64 * h as f64 / w as f64).round() as usize).max(1);
            let scale = if c * r > budget {
                (budget as f64 / (c * r) as f64).sqrt()
            } else {
                1.0
            };
            (
                ((c as f64 * scale).floor() as usize).max(1) as u32,
                ((r as f64 * scale).floor() as usize).max(1) as u32,
            )
        };
        let subs = if cfg.snap.enabled { L2_RATIO as u32 } else { 1 };
        Self { width: w, height: h, cols, rows, subs }
    }

    pub fn l2(&self) -> (u32, u32) {
        (self.cols * L2_RATIO as u32, self.rows * L2_RATIO as u32)
    }

    pub fn mask(&self) -> (u32, u32) {
        let (l2w, l2h) = self.l2();
        (l2w * MASK_PER_CELL, l2h * MASK_PER_CELL)
    }

    pub fn l1_index(&self, c: u32, r: u32) -> usize {
        (r * self.cols + c) as usize
    }

    pub fn cell_of(&self, px: f32, py: f32) -> (u32, u32) {
        let c = ((px / self.width as f32) * self.cols as f32).clamp(0.0, self.cols as f32 - 1.0) as u32;
        let r = ((py / self.height as f32) * self.rows as f32).clamp(0.0, self.rows as f32 - 1.0) as u32;
        (c, r)
    }

    pub fn cell_rect(&self, c: u32, r: u32) -> [f32; 4] {
        let cw = self.width as f32 / self.cols as f32;
        let ch = self.height as f32 / self.rows as f32;
        [c as f32 * cw, r as f32 * ch, (c + 1) as f32 * cw, (r + 1) as f32 * ch]
    }

    pub fn sub_rect(&self, c: u32, r: u32, sc: u32, sr: u32) -> [f32; 4] {
        let cell = self.cell_rect(c, r);
        let sw = (cell[2] - cell[0]) / L2_RATIO as f32;
        let sh = (cell[3] - cell[1]) / L2_RATIO as f32;
        [
            cell[0] + sc as f32 * sw,
            cell[1] + sr as f32 * sh,
            cell[0] + (sc + 1) as f32 * sw,
            cell[1] + (sr + 1) as f32 * sh,
        ]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CellGeo {
    pub sky: f32,
    pub far: f32,
    pub near: f32,
}

impl CellGeo {
    pub fn blocked(&self, cfg: &CellGateCfg) -> bool {
        if !cfg.enabled {
            return false;
        }
        self.sky > cfg.sky_max_frac || self.near < cfg.min_near_frac
    }
}

#[derive(Clone, Copy)]
pub struct View<'a> {
    pub intr: &'a Intrinsics,
    pub pose: &'a Pose,
    pub height: Option<crate::align::HeightFn<'a>>,
}

pub fn cell_geometry(
    pyr: &Pyramid,
    depth: &DepthMap,
    seg: Option<&SegMap>,
    view: Option<View<'_>>,
    far_m: f32,
) -> Vec<CellGeo> {
    let n = (pyr.cols * pyr.rows) as usize;
    let mut out = vec![CellGeo::default(); n];
    for r in 0..pyr.rows {
        for c in 0..pyr.cols {
            let rect = pyr.cell_rect(c, r);
            let (mut sky_cnt, mut far_cnt, mut near_cnt, mut samples) = (0u32, 0u32, 0u32, 0u32);
            let x0 = (rect[0].floor() as u32).min(pyr.width - 1);
            let x1 = (rect[2].ceil() as u32).clamp(x0 + 1, pyr.width);
            let y0 = (rect[1].floor() as u32).min(pyr.height - 1);
            let y1 = (rect[3].ceil() as u32).clamp(y0 + 1, pyr.height);
            let step_x = ((x1 - x0) / 4).max(1);
            let step_y = ((y1 - y0) / 4).max(1);
            let mut y = y0;
            while y < y1 {
                let mut x = x0;
                while x < x1 {
                    samples += 1;
                    let is_sky = if let Some(s) = seg {
                        let sx = ((x as f32 + 0.5) / pyr.width as f32 * s.width as f32)
                            .clamp(0.0, s.width as f32 - 1.0) as u32;
                        let sy = ((y as f32 + 0.5) / pyr.height as f32 * s.height as f32)
                            .clamp(0.0, s.height as f32 - 1.0) as u32;
                        s.is_sky((sy * s.width + sx) as usize)
                    } else if let Some(v) = view {
                        v.intr.ray(v.pose.yaw, x as f32 + 0.5, y as f32 + 0.5)[1] > -1e-4
                    } else {
                        false
                    };
                    if is_sky {
                        sky_cnt += 1;
                    }
                    let d = depth.at_scaled(x as f32 + 0.5, y as f32 + 0.5, pyr.width, pyr.height);
                    if !d.is_finite() || d > far_m {
                        far_cnt += 1;
                    } else {
                        near_cnt += 1;
                    }
                    x += step_x;
                }
                y += step_y;
            }
            let denom = samples.max(1) as f32;
            out[pyr.l1_index(c, r)] = CellGeo {
                sky: sky_cnt as f32 / denom,
                far: far_cnt as f32 / denom,
                near: near_cnt as f32 / denom,
            };
        }
    }
    out
}

pub fn gate_points(
    points: &mut Vec<SemanticPoint>,
    pyr: &Pyramid,
    geo: &[CellGeo],
    cfg: &CellGateCfg,
) -> usize {
    if !cfg.enabled || geo.len() != (pyr.cols * pyr.rows) as usize {
        return 0;
    }
    let mut suppressed = 0usize;
    let mut i = 0;
    while i < points.len() {
        let mut any_blocked = false;
        for (idx, v) in points[i].heat.values.iter_mut().enumerate() {
            if geo.get(idx).is_some_and(|g| g.blocked(cfg)) && *v > -10.0 {
                *v = -1e9;
                any_blocked = true;
            }
        }
        let best = points[i]
            .heat
            .values
            .iter()
            .enumerate()
            .filter(|(idx, v)| v.is_finite() && **v > -1e8 && !geo[*idx].blocked(cfg))
            .max_by(|a, b| a.1.total_cmp(b.1));
        if let Some((best_idx, &best_v)) = best {
            let (bc, br) = (best_idx as u32 % pyr.cols, best_idx as u32 / pyr.cols);
            let rect = pyr.cell_rect(bc, br);
            points[i].px = 0.5 * (rect[0] + rect[2]);
            points[i].py = 0.5 * (rect[1] + rect[3]);
            points[i].score = best_v;
            if any_blocked {
                suppressed += 1;
            }
            i += 1;
        } else {
            suppressed += 1;
            points.swap_remove(i);
        }
    }
    suppressed
}

pub fn snap_points(
    points: &mut [SemanticPoint],
    pyr: &Pyramid,
    depth: &DepthMap,
    view: Option<View<'_>>,
    cfg: &SnapCfg,
) -> usize {
    if !cfg.enabled || pyr.subs < 2 {
        return 0;
    }
    let mut moved = 0usize;
    for pt in points.iter_mut() {
        let (c, r) = pyr.cell_of(pt.px, pt.py);
        let mut candidates = Vec::new();
        for sr in 0..L2_RATIO as u32 {
            for sc in 0..L2_RATIO as u32 {
                let rect = pyr.sub_rect(c, r, sc, sr);
                let cx = 0.5 * (rect[0] + rect[2]);
                let cy = 0.5 * (rect[1] + rect[3]);
                let d = depth.at_scaled(cx, cy, pyr.width, pyr.height);
                if d.is_finite() && d > 0.0 {
                    let g = view.and_then(|v| ground_depth(v.intr, v.pose, cx, cy, v.height, 1e6));
                    let above_ground = g.map_or(true, |gd| d < gd - cfg.ground_slack_m);
                    candidates.push((cx, cy, d, above_ground));
                }
            }
        }
        if candidates.len() < 2 {
            continue;
        }
        let d_cur = depth.at_scaled(pt.px, pt.py, pyr.width, pyr.height);
        let d_min = candidates.iter().map(|x| x.2).fold(f32::INFINITY, f32::min);
        let d_max = candidates.iter().map(|x| x.2).fold(0.0f32, f32::max);
        if d_max <= 0.0 || (d_max - d_min) / d_max < cfg.edge_frac {
            continue;
        }
        if let Some((nx, ny, nd, _)) = candidates
            .into_iter()
            .filter(|(_, _, d, above)| *above && (!d_cur.is_finite() || *d < d_cur * (1.0 - cfg.edge_frac)))
            .min_by(|a, b| a.2.total_cmp(&b.2))
        {
            let _ = nd;
            pt.px = nx;
            pt.py = ny;
            moved += 1;
        }
    }
    moved
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PyramidReport {
    pub l1: (u32, u32),
    pub l2: (u32, u32),
    pub mask: (u32, u32),
    pub blocked_cells: u32,
    pub snapped_points: usize,
}
