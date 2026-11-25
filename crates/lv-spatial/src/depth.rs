//! Shared depth scene model trait and accuracy metrics (`AbsRel`, `delta1`, `RMSE`).

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::frame::{DepthKind, DepthMap, NormalMap, Rgb, SegMap};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneHeads {
    pub normals: bool,
    pub segmentation: bool,
}

impl SceneHeads {
    pub const NONE: SceneHeads = SceneHeads { normals: false, segmentation: false };
    pub const ALL: SceneHeads = SceneHeads { normals: true, segmentation: true };

    pub fn and(self, other: Self) -> Self {
        Self {
            normals: self.normals && other.normals,
            segmentation: self.segmentation && other.segmentation,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DepthScene {
    pub depth: DepthMap,
    pub normals: Option<NormalMap>,
    pub segmentation: Option<SegMap>,
}

impl DepthScene {
    pub fn depth_only(depth: DepthMap) -> Self {
        Self { depth, normals: None, segmentation: None }
    }

    pub fn mask_sky(&mut self) -> u32 {
        let Some(seg) = &self.segmentation else { return 0 };
        let (w, h) = (self.depth.width, self.depth.height);
        let mut n = 0;
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = (
                    (x * seg.width / w.max(1)).min(seg.width - 1),
                    (y * seg.height / h.max(1)).min(seg.height - 1),
                );
                if seg.is_sky((sy * seg.width + sx) as usize) {
                    let d = &mut self.depth.data[(y * w + x) as usize];
                    if d.is_finite() {
                        n += 1;
                    }
                    *d = f32::INFINITY;
                }
            }
        }
        n
    }
}

pub trait DepthModel: Send {
    fn mode(&self) -> &str;
    fn device(&self) -> String {
        "cpu".into()
    }
    fn max_depth_m(&self) -> Option<f32> {
        None
    }
    fn infer(&mut self, rgb: &Rgb) -> Result<DepthMap>;
    fn infer_batch(&mut self, frames: &[&Rgb]) -> Result<Vec<DepthMap>> {
        frames.iter().map(|f| self.infer(f)).collect()
    }
    fn scene_heads(&self) -> SceneHeads {
        SceneHeads::NONE
    }
    fn infer_scene_batch(&mut self, frames: &[&Rgb], _want: SceneHeads) -> Result<Vec<DepthScene>> {
        Ok(self.infer_batch(frames)?.into_iter().map(DepthScene::depth_only).collect())
    }
    fn set_pyramid(&mut self, _cfg: Option<&crate::pyramid::PyramidCfg>) {}
    fn infer_scene(&mut self, rgb: &Rgb, want: SceneHeads) -> Result<DepthScene> {
        let mut v = self.infer_scene_batch(&[rgb], want)?;
        v.pop().ok_or_else(|| anyhow::anyhow!("no scene for the frame"))
    }
    fn stereo_rig(&self) -> Option<crate::stereo::StereoRig> {
        None
    }
    fn infer_pair(&mut self, _left: &Rgb, _right: &Rgb) -> Result<DepthMap> {
        anyhow::bail!("{} is monocular; use stereo pair for SGBM", self.mode())
    }
    fn stereo_stats(&self) -> Option<crate::stereo::StereoStats> {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DepthMetrics {
    pub absrel: f32,
    pub delta1: f32,
    pub rmse_m: f32,
    pub median_scale: f32,
    pub pixels: u32,
}

pub fn depth_metrics(pred: &DepthMap, truth: &DepthMap, max_m: f32, align: bool) -> Option<DepthMetrics> {
    let mut pairs = Vec::new();
    for y in 0..truth.height {
        for x in 0..truth.width {
            let t = truth.at(x, y);
            if !t.is_finite() || t <= 0.0 || t > max_m {
                continue;
            }
            let p = pred.at_scaled(x as f32 + 0.5, y as f32 + 0.5, truth.width, truth.height);
            if p.is_finite() && p > 0.0 {
                pairs.push((p, t));
            }
        }
    }
    if pairs.is_empty() {
        return None;
    }
    let mut ratios: Vec<f32> = pairs.iter().map(|(p, t)| t / p).collect();
    ratios.sort_by(f32::total_cmp);
    let median_scale = ratios[ratios.len() / 2];
    let s = if align { median_scale } else { 1.0 };
    let (mut ar, mut d1, mut se) = (0.0f64, 0u32, 0.0f64);
    for (p, t) in &pairs {
        let p = p * s;
        ar += ((p - t).abs() / t) as f64;
        d1 += ((p / t).max(t / p) < 1.25) as u32;
        se += ((p - t) as f64).powi(2);
    }
    let n = pairs.len() as f64;
    Some(DepthMetrics {
        absrel: (ar / n) as f32,
        delta1: (d1 as f64 / n) as f32,
        rmse_m: (se / n).sqrt() as f32,
        median_scale,
        pixels: pairs.len() as u32,
    })
}

pub fn clamp_metric(map: &mut DepthMap, max_m: f32) {
    map.kind = DepthKind::Metric;
    for d in &mut map.data {
        if d.is_finite() {
            *d = d.clamp(1e-3, max_m);
        }
    }
}
