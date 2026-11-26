//! Fail-closed 3D display privacy guard & convex perspective quadrilateral scanline zeroizer.

use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use lv_core::siglip2::tower::{EpochWatch, VisionAbort, VisionTower};
use lv_screen::{
    bank::{AnchorBank, VisionKind},
    patchify_frame_with,
    screen::{read_pass_guarded, PassGuard, ScreenConfig, ScreenReading},
    Preprocess,
};
use lv_spatial::frame::Rgb;
use lv_spatial::planar::PlanarPatch3D;
use zeroize::Zeroize;

use crate::homography::RectifiedPlanePatch;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanarScreenGuardCfg {
    pub screen: ScreenConfig,
    pub privacy_score_thresh: f32,
    pub margin_px: f32,
    pub fail_closed_without_tower: bool,
}

impl Default for PlanarScreenGuardCfg {
    fn default() -> Self {
        Self {
            screen: ScreenConfig::default(),
            privacy_score_thresh: 0.22,
            margin_px: 4.0,
            fail_closed_without_tower: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenRedactReason {
    VisionPrivacyHit,
    EpochChangedDuringPass,
    ActiveEpochLockdown,
    FailClosedNoTower,
}

#[derive(Clone, Debug)]
pub enum PlanarScreenOutcome {
    Clear(ScreenReading),
    PrivacyRedacted {
        zeroized_pixels: usize,
        reason: ScreenRedactReason,
        trigger_sensor_purge: bool,
    },
}

pub fn zeroize_perspective_quad_rgb(
    frame: &mut Rgb,
    corners_px: &[[f32; 2]; 4],
    margin_px: f32,
) -> usize {
    let w = frame.width as usize;
    let h = frame.height as usize;
    if w == 0 || h == 0 || frame.data.len() < w * h * 3 {
        return 0;
    }
    if corners_px.iter().any(|c| !c[0].is_finite() || !c[1].is_finite()) {
        return 0;
    }

    let diag_px = (corners_px[0][0] - corners_px[2][0])
        .hypot(corners_px[0][1] - corners_px[2][1])
        .max((corners_px[1][0] - corners_px[3][0]).hypot(corners_px[1][1] - corners_px[3][1]));
    let eff_margin = margin_px.max(0.02 * diag_px).max(0.0);

    let mut area2 = 0.0f32;
    for i in 0..4 {
        let j = (i + 1) & 3;
        area2 += corners_px[i][0] * corners_px[j][1] - corners_px[j][0] * corners_px[i][1];
    }
    if area2.abs() < 1e-3 {
        return 0;
    }
    let sign = if area2 >= 0.0 { 1.0f32 } else { -1.0f32 };

    let mut edges = [(0.0f32, 0.0f32, 0.0f32, 0.0f32, 1.0f32); 4];
    let (mut min_x, mut max_x) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut min_y, mut mut_max_y) = (f32::INFINITY, f32::NEG_INFINITY);
    for i in 0..4 {
        let a = corners_px[i];
        let b = corners_px[(i + 1) & 3];
        min_x = min_x.min(a[0]);
        max_x = max_x.max(a[0]);
        min_y = min_y.min(a[1]);
        mut_max_y = mut_max_y.max(a[1]);
        let ex = b[0] - a[0];
        let ey = b[1] - a[1];
        let len = ex.hypot(ey).max(1e-6);
        edges[i] = (a[0], a[1], ex, ey, len);
    }

    let miter_cap = (eff_margin * 2.0).max(2.0);
    let x0 = ((min_x - miter_cap).floor().max(0.0) as usize).min(w - 1);
    let x1 = ((max_x + miter_cap).ceil().max(0.0) as usize).min(w - 1);
    let y0 = ((min_y - miter_cap).floor().max(0.0) as usize).min(h - 1);
    let y1 = ((mut_max_y + miter_cap).ceil().max(0.0) as usize).min(h - 1);

    let mut zeroed_px = 0usize;
    for py_i in y0..=y1 {
        let py = py_i as f32 + 0.5;
        let row_base = py_i * w * 3;
        for px_i in x0..=x1 {
            let px = px_i as f32 + 0.5;
            let inside = edges.iter().all(|&(ax, ay, ex, ey, len)| {
                let signed_dist = sign * (ex * (py - ay) - ey * (px - ax)) / len;
                signed_dist >= -eff_margin
            });
            if inside {
                let idx = row_base + px_i * 3;
                frame.data[idx..idx + 3].zeroize();
                zeroed_px += 1;
            }
        }
    }
    zeroed_px
}

pub fn inspect_and_guard_3d_screen(
    frame_rgb: &mut Rgb,
    rect: &mut RectifiedPlanePatch,
    patch3d: &PlanarPatch3D,
    screen_ctx: Option<(&VisionTower, &AnchorBank)>,
    privacy_epoch: &AtomicU64,
    captured_epoch: u64,
    cfg: &PlanarScreenGuardCfg,
) -> Result<PlanarScreenOutcome> {
    let cur_epoch = privacy_epoch.load(Ordering::Acquire);
    if cur_epoch != captured_epoch {
        rect.wipe();
        let n = zeroize_perspective_quad_rgb(frame_rgb, &patch3d.corners_px, cfg.margin_px);
        return Ok(PlanarScreenOutcome::PrivacyRedacted {
            zeroized_pixels: n,
            reason: ScreenRedactReason::EpochChangedDuringPass,
            trigger_sensor_purge: true,
        });
    }

    let Some((tower, bank)) = screen_ctx else {
        if cur_epoch > 0 || cfg.fail_closed_without_tower {
            rect.wipe();
            let reason = if cur_epoch > 0 {
                ScreenRedactReason::ActiveEpochLockdown
            } else {
                ScreenRedactReason::FailClosedNoTower
            };
            let n = zeroize_perspective_quad_rgb(frame_rgb, &patch3d.corners_px, cfg.margin_px);
            return Ok(PlanarScreenOutcome::PrivacyRedacted {
                zeroized_pixels: n,
                reason,
                trigger_sensor_purge: cur_epoch > 0,
            });
        }
        anyhow::bail!("screen_ctx is required when fail_closed_without_tower is disabled");
    };

    let mut grid = patchify_frame_with(
        &rect.rgb,
        rect.width as usize,
        rect.height as usize,
        cfg.screen.max_patches,
        &Preprocess::SIGLIP2,
    );
    let guard = PassGuard {
        epoch: Some(EpochWatch {
            epoch: privacy_epoch,
            captured: captured_epoch,
        }),
        masked: &[],
    };

    let reading_opt = match read_pass_guarded(tower, &cfg.screen, &mut grid, bank, &guard) {
        Ok(r) => r,
        Err(err) if VisionAbort::of(&err).is_some() => None,
        Err(err) => {
            grid.wipe();
            rect.wipe();
            return Err(err);
        }
    };

    let Some(reading) = reading_opt else {
        grid.wipe();
        rect.wipe();
        let n = zeroize_perspective_quad_rgb(frame_rgb, &patch3d.corners_px, cfg.margin_px);
        return Ok(PlanarScreenOutcome::PrivacyRedacted {
            zeroized_pixels: n,
            reason: ScreenRedactReason::EpochChangedDuringPass,
            trigger_sensor_purge: true,
        });
    };

    let privacy_hit = reading
        .scores
        .iter()
        .any(|s| s.kind == VisionKind::Privacy && s.score >= cfg.privacy_score_thresh);

    if privacy_hit {
        privacy_epoch.fetch_add(1, Ordering::Release);
        grid.wipe();
        rect.wipe();
        let n = zeroize_perspective_quad_rgb(frame_rgb, &patch3d.corners_px, cfg.margin_px);
        return Ok(PlanarScreenOutcome::PrivacyRedacted {
            zeroized_pixels: n,
            reason: ScreenRedactReason::VisionPrivacyHit,
            trigger_sensor_purge: true,
        });
    }

    grid.wipe();
    Ok(PlanarScreenOutcome::Clear(reading))
}
