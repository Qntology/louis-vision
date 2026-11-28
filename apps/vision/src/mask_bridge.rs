//! Slice-scoped mask editing & propagation bridge (`vision-sandbox v1.8`).
//!
//! Connects `lv_spatial::mask_ops` (`PointStore`, `PostprocessParams`, `apply`) to
//! 10-second spatial-deck slices so interactive `+/-` clicks refine a keyframe mask
//! and propagate cleanly across the slice without touching global history.

use std::collections::BTreeMap;

use anyhow::{bail, ensure, Result};
use image::GrayImage;
use lv_spatial::mask_ops::{apply, DataPoint, PointStore, PostprocessParams};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskPostprocessCfg {
    pub holes: u32,
    pub dots: u32,
    pub border_fix: u32,
    pub grow: i32,
}

impl Default for MaskPostprocessCfg {
    fn default() -> Self {
        Self {
            holes: 4,
            dots: 2,
            border_fix: 1,
            grow: 0,
        }
    }
}

impl From<MaskPostprocessCfg> for PostprocessParams {
    fn from(c: MaskPostprocessCfg) -> Self {
        Self {
            holes: c.holes,
            dots: c.dots,
            border_fix: c.border_fix,
            grow: c.grow,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SliceMaskSummary {
    pub slice_idx: u32,
    pub start_frame: u32,
    pub end_frame: u32,
    pub point_count: usize,
    pub propagated_frames: usize,
    pub foreground_pixels_at_seed: usize,
}

/// Manages interactive foreground/background seed clicks and local 10-second slice mask propagation.
pub struct SliceMaskSession {
    pub slice_idx: u32,
    pub start_frame: u32,
    pub end_frame: u32,
    pub width: u32,
    pub height: u32,
    pub points: PointStore,
    pub params: MaskPostprocessCfg,
    masks: BTreeMap<u32, GrayImage>,
}

impl SliceMaskSession {
    pub fn new(slice_idx: u32, start_frame: u32, end_frame: u32, width: u32, height: u32) -> Result<Self> {
        ensure!(width >= 8 && height >= 8, "mask dimensions must be >= 8x8");
        ensure!(end_frame >= start_frame, "end_frame must be >= start_frame");
        Ok(Self {
            slice_idx,
            start_frame,
            end_frame,
            width,
            height,
            points: PointStore::new(),
            params: MaskPostprocessCfg::default(),
            masks: BTreeMap::new(),
        })
    }

    pub fn add_point(&mut self, frame: u32, object_id: u32, positive: bool, x: i32, y: i32) -> Result<()> {
        ensure!(
            (self.start_frame..=self.end_frame).contains(&frame),
            "frame {frame} outside slice range {}..={}",
            self.start_frame,
            self.end_frame
        );
        self.points.add(DataPoint { frame, object_id, positive, x, y });
        Ok(())
    }

    pub fn undo_point(&mut self) -> Option<DataPoint> {
        self.points.remove_last()
    }

    pub fn set_params(&mut self, params: MaskPostprocessCfg) {
        self.params = params;
    }

    /// Synthesizes a binary foreground mask from the `PointStore` positive/negative clicks at `frame`
    /// and applies `lv_spatial::mask_ops::apply` (`holes`, `dots`, `border_fix`, `grow`).
    pub fn build_keyframe_mask(&mut self, frame: u32, object_id: u32, seed_radius_px: i32) -> Result<&GrayImage> {
        let prompts = self.points.prompts(frame, object_id);
        if prompts.coords.is_empty() {
            bail!("no seed points registered for frame={frame} object_id={object_id}");
        }
        let r = seed_radius_px.max(2);
        let r2 = r * r;
        let neg_r2 = (r / 2).max(2).pow(2);
        let (w, h) = (self.width as i32, self.height as i32);
        let mut raw = vec![0u8; (self.width * self.height) as usize];

        for (&[px, py], &label) in prompts.coords.iter().zip(prompts.labels.iter()) {
            let cx = px.round() as i32;
            let cy = py.round() as i32;
            if label == 1 {
                for y in (cy - r).max(0)..=(cy + r).min(h - 1) {
                    for x in (cx - r).max(0)..=(cx + r).min(w - 1) {
                        if (x - cx) * (x - cx) + (y - cy) * (y - cy) <= r2 {
                            raw[(y as u32 * self.width + x as u32) as usize] = 255;
                        }
                    }
                }
            }
        }
        for (&[px, py], &label) in prompts.coords.iter().zip(prompts.labels.iter()) {
            let cx = px.round() as i32;
            let cy = py.round() as i32;
            if label == 0 {
                for y in (cy - r).max(0)..=(cy + r).min(h - 1) {
                    for x in (cx - r).max(0)..=(cx + r).min(w - 1) {
                        if (x - cx) * (x - cx) + (y - cy) * (y - cy) <= neg_r2 {
                            raw[(y as u32 * self.width + x as u32) as usize] = 0;
                        }
                    }
                }
            }
        }

        let base = GrayImage::from_raw(self.width, self.height, raw)
            .ok_or_else(|| anyhow::anyhow!("failed to construct mask image"))?;
        let post = apply(&base, &PostprocessParams::from(self.params.clone()))?;
        self.masks.insert(frame, post);
        Ok(self.masks.get(&frame).expect("inserted"))
    }

    /// Propagates the refined keyframe mask across all frames `[start_frame..=end_frame]` within this slice.
    pub fn propagate_within_slice(
        &mut self,
        seed_frame: u32,
        object_id: u32,
        seed_radius_px: i32,
        dx_per_frame: f32,
        dy_per_frame: f32,
    ) -> Result<SliceMaskSummary> {
        let seed_mask = self.build_keyframe_mask(seed_frame, object_id, seed_radius_px)?.clone();
        let fg_seed = seed_mask.as_raw().iter().filter(|&&v| v > 0).count();
        let (w, h) = (self.width as i32, self.height as i32);

        for f in self.start_frame..=self.end_frame {
            if f == seed_frame {
                continue;
            }
            let dt = f as f32 - seed_frame as f32;
            let shift_x = (dx_per_frame * dt).round() as i32;
            let shift_y = (dy_per_frame * dt).round() as i32;
            let mut shifted = vec![0u8; (self.width * self.height) as usize];
            let src = seed_mask.as_raw();
            for y in 0..h {
                let sy = y - shift_y;
                if sy < 0 || sy >= h {
                    continue;
                }
                for x in 0..w {
                    let sx = x - shift_x;
                    if sx < 0 || sx >= w {
                        continue;
                    }
                    shifted[(y as u32 * self.width + x as u32) as usize] =
                        src[(sy as u32 * self.width + sx as u32) as usize];
                }
            }
            let img = GrayImage::from_raw(self.width, self.height, shifted).expect("valid dimensions");
            self.masks.insert(f, img);
        }

        Ok(SliceMaskSummary {
            slice_idx: self.slice_idx,
            start_frame: self.start_frame,
            end_frame: self.end_frame,
            point_count: self.points.points().len(),
            propagated_frames: self.masks.len(),
            foreground_pixels_at_seed: fg_seed,
        })
    }

    pub fn mask_at(&self, frame: u32) -> Option<&GrayImage> {
        self.masks.get(&frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_mask_bridge_refines_points_via_lv_spatial_and_propagates_within_10s_slice() {
        let mut sess = SliceMaskSession::new(0, 0, 9, 64, 64).unwrap();
        sess.set_params(MaskPostprocessCfg {
            holes: 3,
            dots: 2,
            border_fix: 1,
            grow: 1,
        });
        sess.add_point(0, 1, true, 24, 32).unwrap();
        sess.add_point(0, 1, false, 24, 32).unwrap();
        // Undo the overlapping negative click and add a clean background exclusion outside the blob
        assert!(sess.undo_point().is_some());
        sess.add_point(0, 1, false, 52, 52).unwrap();
        // Out-of-slice frame rejected cleanly
        assert!(sess.add_point(10, 1, true, 10, 10).is_err());

        let summary = sess.propagate_within_slice(0, 1, 8, 1.0, 0.0).unwrap();
        assert_eq!(summary.propagated_frames, 10);
        assert!(summary.foreground_pixels_at_seed > 150);
        let m9 = sess.mask_at(9).expect("frame 9 propagated");
        // Blob center shifted +9 px to x=33, y=32
        assert_eq!(m9.get_pixel(33, 32).0[0], 255);
    }
}
