//! Stereo sandbox keying & unlimited 3D performer tracking (`StereoSpatialTier::StereoSandboxLive`).
//! Wires `lv_spatial::stereo::sgbm` -> `SandboxZKeyTracker` (NaN hole motion recovery + RGB Joint
//! Bilateral `AlphaMatte` + Hungarian 3D tracker) -> `mask_ops::postprocess` -> 21-color ID matte.

use anyhow::Result;
use lv_spatial::camera::{Intrinsics, Pose};
use lv_spatial::fit3::Similarity;
use lv_spatial::frame::{AlphaMatte, DepthMap, Mask, Rgb};
use lv_spatial::mask_ops::palette::object_color;
use lv_spatial::mask_ops::postprocess::{apply as postprocess_mask, PostprocessParams};
use lv_spatial::pyramid::PyramidCfg;
use lv_spatial::stereo::{SgbmCfg, StereoOut, StereoRig};
use lv_spatial::track::{StereoSpatialOut, StereoSpatialPipeline, StereoSpatialTier};
use lv_spatial::zkey::{PerformerTrack3D, SandboxBackground, SandboxKeyOut, SandboxZKeyTracker, ZKeyCfg};

#[derive(Clone, Debug)]
pub struct SandboxFrameOut {
    pub frame_index: u64,
    pub depth: DepthMap,
    pub stereo_valid_ratio: f32,
    pub mask: Mask,
    pub matte: AlphaMatte,
    pub id_matte_rgba: Vec<u8>,
    pub performers: Vec<PerformerTrack3D>,
    pub hole_recovered_px: u32,
}

pub struct StereoSandboxLive {
    pipeline: StereoSpatialPipeline,
    standalone_zkey: SandboxZKeyTracker,
    intr: Intrinsics,
    pose: Pose,
    rig_to_sandbox: Similarity,
    postprocess: PostprocessParams,
    frame_index: u64,
}

impl StereoSandboxLive {
    pub fn new(
        rig: StereoRig,
        sgbm_cfg: SgbmCfg,
        pyramid: Option<PyramidCfg>,
        bg: SandboxBackground,
        zkey_cfg: ZKeyCfg,
        intr: Intrinsics,
        pose: Pose,
        rig_to_sandbox: Similarity,
    ) -> Self {
        let tracker_pipe = SandboxZKeyTracker::new(bg.clone(), zkey_cfg);
        let standalone_zkey = SandboxZKeyTracker::new(bg, zkey_cfg);
        let pipeline = StereoSpatialPipeline::new(rig, sgbm_cfg, pyramid, Some(tracker_pipe));
        Self {
            pipeline,
            standalone_zkey,
            intr,
            pose,
            rig_to_sandbox,
            postprocess: PostprocessParams {
                holes: 2,
                dots: 2,
                border_fix: 0,
                grow: 0,
            },
            frame_index: 0,
        }
    }

    pub fn with_postprocess(mut self, postprocess: PostprocessParams) -> Self {
        self.postprocess = postprocess;
        self
    }

    /// Full zero-SAM3.1 live pipeline: Census-SGBM -> empty-sandbox ΔZ Z-Key -> ID matte & sandbox coords.
    pub fn process_stereo_pair(&mut self, left: &Rgb, right: &Rgb) -> Result<SandboxFrameOut> {
        let StereoSpatialOut {
            stereo: StereoOut { depth, stats, .. },
            matte,
            performers,
            hole_recovered_px,
            ..
        } = self.pipeline.run_tier(
            StereoSpatialTier::StereoSandboxLive,
            left,
            right,
            &self.intr,
            &self.pose,
        )?;
        let valid_ratio = stats.valid_frac();
        let matte = matte.unwrap_or(AlphaMatte {
            width: left.width,
            height: left.height,
            alpha: vec![0u8; (left.width * left.height) as usize],
        });
        let raw_mask = mask_from_alpha(&matte);
        self.finalize_frame(depth, valid_ratio, raw_mask, matte, performers, hole_recovered_px)
    }

    /// Processes a pre-computed metric `DepthMap` directly through `SandboxZKeyTracker` (e.g. calibrated
    /// depth sensors or deterministic synthetic stereo-depth test harnesses).
    pub fn process_calibrated_depth(&mut self, rgb: &Rgb, depth: &DepthMap) -> Result<SandboxFrameOut> {
        let SandboxKeyOut {
            mask,
            matte,
            performers,
            hole_recovered_px,
        } = self
            .standalone_zkey
            .step(rgb, depth, &self.intr, &self.pose, Some(self.pipeline.rig))?;
        let valid_count = depth.data.iter().filter(|d| d.is_finite() && **d > 0.0).count();
        let valid_ratio = valid_count as f32 / depth.data.len().max(1) as f32;
        self.finalize_frame(
            depth.clone(),
            valid_ratio,
            mask,
            matte,
            performers,
            hole_recovered_px,
        )
    }

    fn finalize_frame(
        &mut self,
        depth: DepthMap,
        stereo_valid_ratio: f32,
        raw_mask: Mask,
        matte: AlphaMatte,
        mut performers: Vec<PerformerTrack3D>,
        hole_recovered_px: u32,
    ) -> Result<SandboxFrameOut> {
        let refined_mask = if self.postprocess != PostprocessParams::default() {
            let gray = postprocess_mask(&raw_mask.to_gray(), &self.postprocess)?;
            Mask::from_gray(&gray)
        } else {
            raw_mask
        };

        for p in &mut performers {
            let world = self.rig_to_sandbox.apply([
                p.pos_m[0] as f64,
                p.pos_m[1] as f64,
                p.pos_m[2] as f64,
            ]);
            p.pos_m = [world[0] as f32, world[1] as f32, world[2] as f32];
        }

        let id_matte_rgba = render_id_matte_rgba(&matte, &refined_mask, &performers);
        self.frame_index += 1;

        Ok(SandboxFrameOut {
            frame_index: self.frame_index,
            depth,
            stereo_valid_ratio,
            mask: refined_mask,
            matte,
            id_matte_rgba,
            performers,
            hole_recovered_px,
        })
    }
}

fn mask_from_alpha(matte: &AlphaMatte) -> Mask {
    let bits = matte.alpha.iter().map(|&a| a >= 128).collect();
    Mask {
        width: matte.width,
        height: matte.height,
        bits,
    }
}

/// Colors foreground alpha pixels using deterministic 21-color cyclic ID palette (`mask_ops::palette`).
fn render_id_matte_rgba(matte: &AlphaMatte, mask: &Mask, performers: &[PerformerTrack3D]) -> Vec<u8> {
    let (w, h) = (matte.width.max(1), matte.height.max(1));
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let my = ((y as u64 * mask.height as u64) / h as u64).min(mask.height.saturating_sub(1) as u64) as u32;
        for x in 0..w {
            let idx = (y * w + x) as usize;
            let a = matte.alpha[idx];
            if a == 0 {
                continue;
            }
            let mx = ((x as u64 * mask.width as u64) / w as u64).min(mask.width.saturating_sub(1) as u64) as u32;
            if mask.width > 0 && mask.height > 0 && !mask.get(mx, my) && a < 64 {
                continue;
            }
            let pid = performers
                .iter()
                .find(|p| mx >= p.bbox_px[0] && mx <= p.bbox_px[2] && my >= p.bbox_px[1] && my <= p.bbox_px[3])
                .or_else(|| performers.first())
                .map_or(0, |p| p.id);
            let rgb = object_color(pid);
            let o = idx * 4;
            rgba[o] = rgb[0];
            rgba[o + 1] = rgb[1];
            rgba[o + 2] = rgb[2];
            rgba[o + 3] = a;
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use lv_spatial::frame::DepthKind;
    use lv_spatial::stereo::sgbm;
    use crate::rig_capture::{decode_sbs_stereo_pair, RigSource};
    use crate::synthetic_textured_sbs;

    fn identity_sim() -> Similarity {
        Similarity {
            s: 1.0,
            r: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            t: [0.0, 0.0, 0.0],
        }
    }

    #[test]
    fn sgbm_plus_zkey_stereo_pair_live_pipeline() {
        let rig = StereoRig { focal_px: 300.0, baseline_m: 0.12, subpixel_px: 0.15 };
        let mut cfg = SgbmCfg::default();
        cfg.per_l2 = 8;
        cfg.speckle_size = 8;
        let src = RigSource::UsbUvcUncompressed {
            device_id: "uvc-test".into(),
            width: 576,
            height: 192,
            fps: 60,
        };
        let (bg_l, bg_r) = decode_sbs_stereo_pair(&synthetic_textured_sbs(288, 192, None, 6, 6), 100.0, &src).unwrap();
        let bg_out = sgbm(&bg_l, &bg_r, &rig, &cfg, None).unwrap();
        let bg_refs: Vec<&DepthMap> = (0..30).map(|_| &bg_out.depth).collect();
        let bg = SandboxBackground::capture_median(&bg_refs, Some(bg_l), 0.03).unwrap();

        let intr = Intrinsics { width: 288, height: 192, fov_deg: 70.0, pitch_deg: 0.0, roll_deg: 0.0 };
        let pose = Pose { x: 0.0, z: 0.0, yaw: 0.0, eye_m: 1.5 };
        let mut live = StereoSandboxLive::new(rig, cfg, None, bg, ZKeyCfg::default(), intr, pose, identity_sim());

        let (fg_l, fg_r) = decode_sbs_stereo_pair(
            &synthetic_textured_sbs(288, 192, Some([112, 48, 176, 152]), 6, 14),
            100.0,
            &src,
        )
        .unwrap();
        let frame = live.process_stereo_pair(&fg_l, &fg_r).unwrap();
        assert_eq!(frame.frame_index, 1);
        assert_eq!(frame.matte.width, 288);
        assert_eq!(frame.matte.height, 192);
        assert_eq!(frame.id_matte_rgba.len(), 288 * 192 * 4);
        assert!(frame.stereo_valid_ratio >= 0.85, "valid ratio={}", frame.stereo_valid_ratio);
        assert!(!frame.performers.is_empty(), "performer must be detected via SGBM+ZKey");
    }

    #[test]
    fn crossing_performers_in_depth_keep_distinct_ids_and_recover_holes() {
        let rig = StereoRig { focal_px: 300.0, baseline_m: 0.12, subpixel_px: 0.15 };
        let (dw, dh) = (60u32, 40u32);
        let bg_depth = DepthMap { width: dw, height: dh, kind: DepthKind::Metric, data: vec![6.0; (dw * dh) as usize] };
        let bg_refs: Vec<&DepthMap> = (0..30).map(|_| &bg_depth).collect();
        let bg_rgb = Rgb::new(120, 80, vec![30u8; 120 * 80 * 3]);
        let bg = SandboxBackground::capture_median(&bg_refs, Some(bg_rgb), 0.03).unwrap();

        let intr = Intrinsics { width: 120, height: 80, fov_deg: 70.0, pitch_deg: 0.0, roll_deg: 0.0 };
        let pose = Pose { x: 0.0, z: 0.0, yaw: 0.0, eye_m: 1.5 };
        let mut live = StereoSandboxLive::new(rig, SgbmCfg::default(), None, bg, ZKeyCfg::default(), intr, pose, identity_sim());

        let mut d = DepthMap { width: dw, height: dh, kind: DepthKind::Metric, data: vec![6.0; (dw * dh) as usize] };
        let mut rgb_buf = vec![30u8; 120 * 80 * 3];
        for y in 8..32 {
            for x in 14..24 {
                d.data[(y * dw + x) as usize] = 2.0;
                for ry in (y * 2)..(y * 2 + 2) {
                    for rx in (x * 2)..(x * 2 + 2) {
                        let i = ((ry * 120 + rx) * 3) as usize;
                        rgb_buf[i..i + 3].copy_from_slice(&[230, 230, 230]);
                    }
                }
            }
            for x in 24..34 {
                d.data[(y * dw + x) as usize] = 4.2;
                for ry in (y * 2)..(y * 2 + 2) {
                    for rx in (x * 2)..(x * 2 + 2) {
                        let i = ((ry * 120 + rx) * 3) as usize;
                        rgb_buf[i..i + 3].copy_from_slice(&[230, 230, 230]);
                    }
                }
            }
        }
        d.data[(15 * dw + 18) as usize] = f32::NAN;
        d.data[(16 * dw + 18) as usize] = f32::NAN;
        let rgb = Rgb::new(120, 80, rgb_buf);

        let f1 = live.process_calibrated_depth(&rgb, &d).unwrap();
        assert!(f1.hole_recovered_px >= 2);
        assert_eq!(f1.performers.len(), 2, "identical shirts at Z=2.0m vs 4.2m must not merge");
        let ids1: BTreeSet<u32> = f1.performers.iter().map(|p| p.id).collect();
        let f2 = live.process_calibrated_depth(&rgb, &d).unwrap();
        let ids2: BTreeSet<u32> = f2.performers.iter().map(|p| p.id).collect();
        assert_eq!(ids1.len(), 2);
        assert_eq!(ids1, ids2, "3D track IDs must persist across ticks");
    }

    #[test]
    fn unlimited_performer_clusters_above_sixteen() {
        let rig = StereoRig { focal_px: 300.0, baseline_m: 0.12, subpixel_px: 0.15 };
        let (w, h) = (200u32, 20u32);
        let bg_d = DepthMap { width: w, height: h, kind: DepthKind::Metric, data: vec![8.0; (w * h) as usize] };
        let bg_refs: Vec<&DepthMap> = (0..30).map(|_| &bg_d).collect();
        let bg = SandboxBackground::capture_median(&bg_refs, None, 0.02).unwrap();
        let mut cur = bg_d;
        for k in 0..20u32 {
            let x0 = k * 10 + 1;
            for y in 4..16 {
                for x in x0..(x0 + 4) {
                    cur.data[(y * w + x) as usize] = 2.5;
                }
            }
        }
        let rgb = Rgb::new(w, h, vec![120u8; (w * h * 3) as usize]);
        let intr = Intrinsics { width: w, height: h, fov_deg: 90.0, pitch_deg: 0.0, roll_deg: 0.0 };
        let pose = Pose { x: 0.0, z: 0.0, yaw: 0.0, eye_m: 1.5 };
        let zcfg = ZKeyCfg { border_fix: 0, hole_fill: 0, dot_remove: 0, min_cluster_px: 4, ..ZKeyCfg::default() };
        let mut live = StereoSandboxLive::new(rig, SgbmCfg::default(), None, bg, zcfg, intr, pose, identity_sim())
            .with_postprocess(PostprocessParams::default());

        let out = live.process_calibrated_depth(&rgb, &cur).unwrap();
        assert_eq!(out.performers.len(), 20, "must track >= 20 simultaneous targets without 16-object cap");
    }
}
