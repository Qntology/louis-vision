//! Per-step multi-feature telemetry vector extracted from mask, pyramid geometry, and tracker.

use serde::{Deserialize, Serialize};

use crate::camera::{Intrinsics, Pose};
use crate::frame::{DepthMap, Mask};
use crate::pyramid::{cell_geometry, CellGateCfg, Pyramid};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Features {
    pub has_mask: bool,
    pub area_frac: f32,
    pub iou_prev: Option<f32>,
    pub sky_frac: f32,
    pub near_frac: f32,
    pub depth_median_m: Option<f32>,
    pub contact_range_m: Option<f32>,
    pub blocked_cells: u32,
    pub valid: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlagCfg {
    pub min_area: f32,
    pub max_sky: f32,
    pub min_near: f32,
    pub min_iou: f32,
}

impl Default for FlagCfg {
    fn default() -> Self {
        Self {
            min_area: 1e-4,
            max_sky: 0.5,
            min_near: 0.25,
            min_iou: 0.2,
        }
    }
}

pub struct FusionInput<'a> {
    pub mask: Option<&'a Mask>,
    pub iou_prev: Option<f32>,
    pub depth: Option<&'a DepthMap>,
    pub pyr: Option<&'a Pyramid>,
    pub intr: Option<&'a Intrinsics>,
    pub pose: Option<&'a Pose>,
    pub far_m: f32,
}

pub fn cell_coverage(m: &Mask, c: u32, r: u32, cols: u32, rows: u32) -> f32 {
    let x0 = (c * m.width / cols.max(1)).min(m.width);
    let x1 = ((c + 1) * m.width / cols.max(1)).min(m.width);
    let y0 = (r * m.height / rows.max(1)).min(m.height);
    let y1 = ((r + 1) * m.height / rows.max(1)).min(m.height);
    let (mut hit, mut tot) = (0u32, 0u32);
    for y in y0..y1 {
        for x in x0..x1 {
            tot += 1;
            hit += m.get(x, y) as u32;
        }
    }
    if tot == 0 { 0.0 } else { hit as f32 / tot as f32 }
}

pub fn features(inp: &FusionInput<'_>) -> Features {
    let has_mask = inp.mask.is_some();
    let area_frac = inp.mask.map_or(0.0, |m| {
        m.area() as f32 / (m.width.max(1) * m.height.max(1)) as f32
    });
    let depth_median_m = inp.depth.and_then(|d| inp.mask.and_then(|m| d.median_in(m)));
    let (sky_frac, near_frac, blocked_cells) = match (inp.depth, inp.pyr) {
        (Some(d), Some(p)) => {
            let geo = cell_geometry(p, d, None, None, inp.far_m);
            let n = geo.len().max(1) as f32;
            let cfg = CellGateCfg { far_m: inp.far_m, ..CellGateCfg::default() };
            let sky = geo.iter().map(|g| g.sky).sum::<f32>() / n;
            let near = geo.iter().map(|g| g.near).sum::<f32>() / n;
            let blk = geo.iter().filter(|g| g.blocked(&cfg)).count() as u32;
            (sky, near, blk)
        }
        _ => (0.0, 1.0, 0),
    };
    let contact_range_m = match (inp.intr, inp.pose, inp.mask) {
        (Some(i), Some(po), Some(m)) => {
            crate::align::contact_range(i, po, m, None, 0.3, inp.far_m).map(|x| x.0)
        }
        _ => None,
    };
    let valid = has_mask && area_frac > 0.0 && near_frac > 0.1;
    Features {
        has_mask,
        area_frac,
        iou_prev: inp.iou_prev,
        sky_frac,
        near_frac,
        depth_median_m,
        contact_range_m,
        blocked_cells,
        valid,
    }
}

pub fn flags(f: &Features, cfg: &FlagCfg) -> Vec<&'static str> {
    let mut out = Vec::new();
    if !f.has_mask || f.area_frac < cfg.min_area {
        out.push("no_mask");
    }
    if f.sky_frac > cfg.max_sky {
        out.push("sky_high");
    }
    if f.near_frac < cfg.min_near {
        out.push("near_low");
    }
    if f.iou_prev.is_some_and(|v| v < cfg.min_iou) {
        out.push("iou_drop");
    }
    out
}

pub fn numeric(f: &Features) -> Vec<(&'static str, f64)> {
    vec![
        ("area_frac", f.area_frac as f64),
        ("sky_frac", f.sky_frac as f64),
        ("near_frac", f.near_frac as f64),
        ("depth_median_m", f.depth_median_m.unwrap_or(f32::NAN) as f64),
        ("contact_range_m", f.contact_range_m.unwrap_or(f32::NAN) as f64),
        ("blocked_cells", f.blocked_cells as f64),
    ]
}
