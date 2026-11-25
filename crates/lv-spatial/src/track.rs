//! Spatial tracking (`MaskTracker`, `RegionTracker`) and zero-SAM3.1 stereo spatial tier dispatch
//! (`StereoSpatialTier::{StereoFastTick, StereoMetricGrasp, StereoStageLive}`).

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::camera::{Intrinsics, Pose};
use crate::frame::{AlphaMatte, DepthMap, Mask, Rgb};
use crate::locate::SemanticPoint;
use crate::pyramid::PyramidCfg;
use crate::stereo::{sgbm, SgbmCfg, StereoOut, StereoRig};
use crate::zkey::{PerformerTrack3D, StageKeyOut, StageZKeyTracker};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointPrompt {
    pub label: String,
    pub px: f32,
    pub py: f32,
    pub score: f32,
}

impl From<&SemanticPoint> for PointPrompt {
    fn from(p: &SemanticPoint) -> Self {
        Self {
            label: p.label.clone(),
            px: p.px,
            py: p.py,
            score: p.score,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrackedMask {
    pub id: u32,
    pub label: String,
    pub mask: Mask,
    pub cx: f32,
    pub cy: f32,
    pub bbox: [f32; 4],
    pub area_frac: f32,
    pub score: f32,
    pub iou_prev: Option<f32>,
    pub age: u32,
    pub propagated: bool,
}

pub trait MaskTracker: Send {
    fn mode(&self) -> &str;
    fn reset(&mut self);
    fn update(
        &mut self,
        rgb: &Rgb,
        depth: Option<&DepthMap>,
        prompts: &[PointPrompt],
    ) -> Result<Vec<TrackedMask>>;
    fn prepare(&mut self, _frames: &[&Rgb]) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegionCfg {
    pub luma_tol: f32,
    pub chroma_tol: f32,
    pub depth_tol_frac: f32,
    pub max_frac: f32,
}

impl Default for RegionCfg {
    fn default() -> Self {
        Self {
            luma_tol: 28.0,
            chroma_tol: 24.0,
            depth_tol_frac: 0.22,
            max_frac: 0.35,
        }
    }
}

pub struct RegionTracker {
    pub cfg: RegionCfg,
    prev: Option<Mask>,
    next_id: u32,
}

impl RegionTracker {
    pub fn new(cfg: RegionCfg) -> Self {
        Self { cfg, prev: None, next_id: 1 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtentClamp {
    pub half_w_m: f32,
    pub half_h_m: f32,
}

impl MaskTracker for RegionTracker {
    fn mode(&self) -> &str {
        "region-grow"
    }

    fn reset(&mut self) {
        self.prev = None;
        self.next_id = 1;
    }

    fn update(
        &mut self,
        rgb: &Rgb,
        depth: Option<&DepthMap>,
        prompts: &[PointPrompt],
    ) -> Result<Vec<TrackedMask>> {
        let mut out = Vec::new();
        let (w, h) = (rgb.width, rgb.height);
        for p in prompts {
            let sx = (p.px.round() as u32).min(w.saturating_sub(1));
            let sy = (p.py.round() as u32).min(h.saturating_sub(1));
            let seed_px = rgb.px(sx, sy);
            let seed_luma = 0.299 * seed_px[0] as f32 + 0.587 * seed_px[1] as f32 + 0.114 * seed_px[2] as f32;
            let seed_d = depth.map(|d| d.at_scaled(p.px, p.py, w, h)).filter(|d| d.is_finite() && *d > 0.0);

            let mut mask = Mask::empty(w, h);
            let max_px = ((w * h) as f32 * self.cfg.max_frac) as u32;
            let mut stack = vec![(sx, sy)];
            mask.set(sx, sy);
            let mut count = 1u32;

            while let Some((x, y)) = stack.pop() {
                if count >= max_px {
                    break;
                }
                for (nx, ny) in [
                    (x.saturating_sub(1), y),
                    ((x + 1).min(w - 1), y),
                    (x, y.saturating_sub(1)),
                    (x, (y + 1).min(h - 1)),
                ] {
                    if mask.get(nx, ny) {
                        continue;
                    }
                    let c = rgb.px(nx, ny);
                    let luma = 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
                    let dc = ((c[0] as f32 - seed_px[0] as f32).abs()
                        + (c[1] as f32 - seed_px[1] as f32).abs()
                        + (c[2] as f32 - seed_px[2] as f32).abs())
                        / 3.0;
                    if (luma - seed_luma).abs() > self.cfg.luma_tol || dc > self.cfg.chroma_tol {
                        continue;
                    }
                    if let (Some(sd), Some(dmap)) = (seed_d, depth) {
                        let nd = dmap.at_scaled(nx as f32 + 0.5, ny as f32 + 0.5, w, h);
                        if nd.is_finite() && ((nd - sd).abs() / sd) > self.cfg.depth_tol_frac {
                            continue;
                        }
                    }
                    mask.set(nx, ny);
                    count += 1;
                    stack.push((nx, ny));
                }
            }

            let iou_prev = self.prev.as_ref().map(|pm| mask.iou(pm));
            let (cx, cy) = mask.centroid().unwrap_or((p.px, p.py));
            let bb = mask.bbox().unwrap_or([sx, sy, sx, sy]);
            let id = self.next_id;
            self.next_id += 1;
            self.prev = Some(mask.clone());
            out.push(TrackedMask {
                id,
                label: p.label.clone(),
                mask,
                cx: cx / w.max(1) as f32,
                cy: cy / h.max(1) as f32,
                bbox: [
                    bb[0] as f32 / w.max(1) as f32,
                    bb[1] as f32 / h.max(1) as f32,
                    bb[2] as f32 / w.max(1) as f32,
                    bb[3] as f32 / h.max(1) as f32,
                ],
                area_frac: count as f32 / (w * h).max(1) as f32,
                score: p.score,
                iou_prev,
                age: 1,
                propagated: false,
            });
        }
        Ok(out)
    }
}

/// Zero-SAM3.1 stereo spatial tier replacing all heavyweight neural segmentation in live & robotics paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StereoSpatialTier {
    /// Navigation / exploration tick: Census-SGBM at `per_l2 <= 4`, L1 pyramid + RegionTracker.
    StereoFastTick,
    /// Manipulation / placement grasp tick: full `per_l2 = 8` Census-SGBM + 3D ItemPose/ContactRange.
    StereoMetricGrasp,
    /// Live 60fps empty-stage broadcast & concert XR: Census-SGBM + Z-Keying + Bilateral AlphaMatte + 3D Hungarian tracker.
    StereoStageLive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StereoSpatialOut {
    pub tier: StereoSpatialTier,
    pub stereo: StereoOut,
    pub matte: Option<AlphaMatte>,
    pub performers: Vec<PerformerTrack3D>,
    pub hole_recovered_px: u32,
}

pub struct StereoSpatialPipeline {
    pub rig: StereoRig,
    pub cfg: SgbmCfg,
    pub pyramid: Option<PyramidCfg>,
    pub stage_tracker: Option<StageZKeyTracker>,
}

impl StereoSpatialPipeline {
    pub fn new(
        rig: StereoRig,
        cfg: SgbmCfg,
        pyramid: Option<PyramidCfg>,
        stage_tracker: Option<StageZKeyTracker>,
    ) -> Self {
        Self { rig, cfg, pyramid, stage_tracker }
    }

    pub fn run_tier(
        &mut self,
        tier: StereoSpatialTier,
        left: &Rgb,
        right: &Rgb,
        intr: &Intrinsics,
        pose: &Pose,
    ) -> Result<StereoSpatialOut> {
        let mut cfg = self.cfg;
        cfg.per_l2 = match tier {
            StereoSpatialTier::StereoFastTick => cfg.per_l2.min(4),
            StereoSpatialTier::StereoMetricGrasp => cfg.per_l2.max(8),
            StereoSpatialTier::StereoStageLive => cfg.per_l2.clamp(4, 8),
        };
        let stereo = sgbm(left, right, &self.rig, &cfg, self.pyramid.as_ref())?;
        let mut matte = None;
        let mut performers = Vec::new();
        let mut hole_recovered_px = 0u32;

        if tier == StereoSpatialTier::StereoStageLive {
            if let Some(zkey) = &mut self.stage_tracker {
                let StageKeyOut {
                    matte: m,
                    performers: p,
                    hole_recovered_px: h,
                    ..
                } = zkey.step(left, &stereo.depth, intr, pose, Some(self.rig))?;
                matte = Some(m);
                performers = p;
                hole_recovered_px = h;
            }
        }

        Ok(StereoSpatialOut {
            tier,
            stereo,
            matte,
            performers,
            hole_recovered_px,
        })
    }
}

/// Thin `MaskTracker` implementation switching between zero-SAM3.1 `StereoSpatialTier` modes.
pub struct StereoMaskTracker {
    pub tier: StereoSpatialTier,
    region: RegionTracker,
    stage: Option<StageZKeyTracker>,
    pub extent_clamp: Option<ExtentClamp>,
    pub intr: Intrinsics,
    pub pose: Pose,
}

impl StereoMaskTracker {
    pub fn new(tier: StereoSpatialTier, intr: Intrinsics, pose: Pose) -> Self {
        Self {
            tier,
            region: RegionTracker::new(RegionCfg::default()),
            stage: None,
            extent_clamp: None,
            intr,
            pose,
        }
    }

    pub fn with_stage_tracker(mut self, tracker: StageZKeyTracker) -> Self {
        self.stage = Some(tracker);
        self
    }

    pub fn set_tier(&mut self, tier: StereoSpatialTier) {
        self.tier = tier;
    }
}

impl MaskTracker for StereoMaskTracker {
    fn mode(&self) -> &str {
        match self.tier {
            StereoSpatialTier::StereoFastTick => "stereo-fast-tick",
            StereoSpatialTier::StereoMetricGrasp => "stereo-metric-grasp",
            StereoSpatialTier::StereoStageLive => "stereo-stage-live",
        }
    }

    fn reset(&mut self) {
        self.region.reset();
    }

    fn update(
        &mut self,
        rgb: &Rgb,
        depth: Option<&DepthMap>,
        prompts: &[PointPrompt],
    ) -> Result<Vec<TrackedMask>> {
        if self.tier == StereoSpatialTier::StereoStageLive {
            if let (Some(zkey), Some(d)) = (&mut self.stage, depth) {
                let key = zkey.step(rgb, d, &self.intr, &self.pose, None)?;
                let mut out = Vec::with_capacity(key.performers.len());
                for p in key.performers {
                    out.push(TrackedMask {
                        id: p.id,
                        label: prompts.first().map_or_else(|| "performer".to_string(), |pr| pr.label.clone()),
                        mask: key.mask.clone(),
                        cx: ((p.bbox_px[0] + p.bbox_px[2]) as f32 * 0.5) / d.width.max(1) as f32,
                        cy: ((p.bbox_px[1] + p.bbox_px[3]) as f32 * 0.5) / d.height.max(1) as f32,
                        bbox: [
                            p.bbox_px[0] as f32 / d.width.max(1) as f32,
                            p.bbox_px[1] as f32 / d.height.max(1) as f32,
                            p.bbox_px[2] as f32 / d.width.max(1) as f32,
                            p.bbox_px[3] as f32 / d.height.max(1) as f32,
                        ],
                        area_frac: p.pixel_count as f32 / (d.width * d.height).max(1) as f32,
                        score: 1.0,
                        iou_prev: None,
                        age: 1,
                        propagated: false,
                    });
                }
                return Ok(out);
            }
        }
        // FastTick & MetricGrasp use weighted RGB+depth region growing optionally tightened by metric postprocessing.
        let mut masks = self.region.update(rgb, depth, prompts)?;
        if self.tier == StereoSpatialTier::StereoMetricGrasp {
            let params = crate::mask_ops::postprocess::PostprocessParams {
                holes: 2,
                dots: 2,
                border_fix: 0,
                grow: 0,
            };
            for tm in &mut masks {
                if let Ok(refined) = crate::mask_ops::postprocess::apply(&tm.mask.to_gray(), &params) {
                    tm.mask = Mask::from_gray(&refined);
                }
            }
        }
        Ok(masks)
    }
}
