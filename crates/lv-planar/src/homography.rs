//! Closed-form 3x3 metric homography `H = K * [r1_c | r2_c | t_c]` and bilinear unwarping.

use image::RgbImage;
use lv_spatial::camera::{Intrinsics, Pose};
use lv_spatial::egoexo::inv3;
use lv_spatial::fit3::{M3, V3};
use lv_spatial::frame::Rgb;
use lv_spatial::planar::PlanarPatch3D;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

pub const DEFAULT_CROP_MAX_SIDE_PX: u32 = 2016;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanarHomography {
    pub h_rect_to_img: M3,
    pub h_img_to_rect: M3,
    pub origin_tl_w: V3,
    pub step_u_w: V3,
    pub step_v_w: V3,
    pub px_per_mm: f32,
    pub margin_mm: f32,
    pub width_px: u32,
    pub height_px: u32,
}

impl PlanarHomography {
    #[inline]
    pub fn rect_to_img(&self, u: f64, v: f64) -> Option<(f64, f64, f64)> {
        let m = &self.h_rect_to_img;
        let w = m[2][0] * u + m[2][1] * v + m[2][2];
        if !w.is_finite() || w <= 1e-6 {
            return None;
        }
        Some((
            (m[0][0] * u + m[0][1] * v + m[0][2]) / w,
            (m[1][0] * u + m[1][1] * v + m[1][2]) / w,
            w,
        ))
    }

    #[inline]
    pub fn img_to_rect(&self, px: f64, py: f64) -> Option<(f64, f64)> {
        let m = &self.h_img_to_rect;
        let w = m[2][0] * px + m[2][1] * py + m[2][2];
        if !w.is_finite() || w.abs() <= 1e-12 {
            return None;
        }
        Some((
            (m[0][0] * px + m[0][1] * py + m[0][2]) / w,
            (m[1][0] * px + m[1][1] * py + m[1][2]) / w,
        ))
    }
}

pub struct RectifiedPlanePatch {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
    pub homography: PlanarHomography,
}

impl RectifiedPlanePatch {
    pub fn to_rgb_image(&self) -> Option<RgbImage> {
        RgbImage::from_raw(self.width, self.height, self.rgb.clone())
    }

    #[inline]
    pub fn wipe(&mut self) {
        self.rgb.zeroize();
    }
}

impl Drop for RectifiedPlanePatch {
    fn drop(&mut self) {
        self.rgb.zeroize();
    }
}

pub fn compute_planar_homography(
    patch: &PlanarPatch3D,
    intr: &Intrinsics,
    pose: &Pose,
    target_px_per_mm: f32,
    margin_mm: f32,
    max_side_px: u32,
) -> Option<PlanarHomography> {
    let cap_px = max_side_px.clamp(32, DEFAULT_CROP_MAX_SIDE_PX);
    let margin = margin_mm.max(0.0);
    let total_w_mm = (patch.width_mm + 2.0 * margin).max(4.0);
    let total_h_mm = (patch.height_mm + 2.0 * margin).max(4.0);

    let obs_max = patch.effective_px_per_mm_u.max(patch.effective_px_per_mm_v).max(0.5);
    let mut scale = target_px_per_mm.min(obs_max * 1.5).max(1.0);
    let max_dim_mm = total_w_mm.max(total_h_mm);
    if max_dim_mm * scale > cap_px as f32 {
        scale = cap_px as f32 / max_dim_mm;
    }

    let width_px = ((total_w_mm * scale).round() as u32).clamp(16, cap_px);
    let height_px = ((total_h_mm * scale).round() as u32).clamp(16, cap_px);
    let delta_m = 1e-3 / scale as f64;

    let half_u_m = 0.5 * (width_px.saturating_sub(1) as f64) * delta_m;
    let half_v_m = 0.5 * (height_px.saturating_sub(1) as f64) * delta_m;
    let origin_tl_w: V3 = [
        patch.center_w[0] - half_u_m * patch.tangent_u_w[0] - half_v_m * patch.tangent_v_w[0],
        patch.center_w[1] - half_u_m * patch.tangent_u_w[1] - half_v_m * patch.tangent_v_w[1],
        patch.center_w[2] - half_u_m * patch.tangent_u_w[2] - half_v_m * patch.tangent_v_w[2],
    ];
    let step_u_w: V3 = patch.tangent_u_w.map(|c| c * delta_m);
    let step_v_w: V3 = patch.tangent_v_w.map(|c| c * delta_m);

    let (fwd, right, up) = intr.camera_axes(pose.yaw);
    let r_vec: V3 = right.map(|v| v as f64);
    let d_vec: V3 = up.map(|v| -(v as f64));
    let f_vec: V3 = fwd.map(|v| v as f64);
    let cam_o: V3 = [pose.x as f64, pose.eye_m as f64, pose.z as f64];
    let t_w: V3 = [
        origin_tl_w[0] - cam_o[0],
        origin_tl_w[1] - cam_o[1],
        origin_tl_w[2] - cam_o[2],
    ];

    let f_px = intr.focal_px();
    let cx = 0.5 * intr.width.max(1) as f64;
    let cy = 0.5 * intr.height.max(1) as f64;
    let dot = |a: V3, b: V3| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cols_cam = [step_u_w, step_v_w, t_w].map(|col| [dot(r_vec, col), dot(d_vec, col), dot(f_vec, col)]);

    let mut h_r2i: M3 = [[0.0; 3]; 3];
    for c in 0..3 {
        h_r2i[0][c] = f_px * cols_cam[c][0] + cx * cols_cam[c][2];
        h_r2i[1][c] = f_px * cols_cam[c][1] + cy * cols_cam[c][2];
        h_r2i[2][c] = cols_cam[c][2];
    }

    for &(u, v) in &[
        (0.0, 0.0),
        ((width_px - 1) as f64, 0.0),
        ((width_px - 1) as f64, (height_px - 1) as f64),
        (0.0, (height_px - 1) as f64),
    ] {
        let z_c = h_r2i[2][0] * u + h_r2i[2][1] * v + h_r2i[2][2];
        if !z_c.is_finite() || z_c <= 0.02 {
            return None;
        }
    }

    let h_i2r = inv3(&h_r2i)?;
    Some(PlanarHomography {
        h_rect_to_img: h_r2i,
        h_img_to_rect: h_i2r,
        origin_tl_w,
        step_u_w,
        step_v_w,
        px_per_mm: scale,
        margin_mm: margin,
        width_px,
        height_px,
    })
}

pub fn unwarp_planar_patch(src: &Rgb, homography: PlanarHomography) -> RectifiedPlanePatch {
    let dst_w = homography.width_px as usize;
    let dst_h = homography.height_px as usize;
    let mut out = vec![255u8; dst_w * dst_h * 3];
    let src_w = src.width as usize;
    let src_h = src.height as usize;
    if src_w < 2 || src_h < 2 || src.data.len() < src_w * src_h * 3 {
        return RectifiedPlanePatch {
            width: homography.width_px,
            height: homography.height_px,
            rgb: out,
            homography,
        };
    }

    let h = homography.h_rect_to_img;
    let max_x = (src_w - 1) as f64;
    let max_y = (src_h - 1) as f64;
    let src_bytes = src.data.as_slice();

    out.par_chunks_mut(dst_w * 3)
        .enumerate()
        .for_each(|(v_idx, row_dst)| {
            let vf = v_idx as f64;
            let mut num_x = h[0][1] * vf + h[0][2];
            let mut num_y = h[1][1] * vf + h[1][2];
            let mut den = h[2][1] * vf + h[2][2];

            for u_idx in 0..dst_w {
                let dst_off = u_idx * 3;
                if den > 1e-6 {
                    let inv_w = 1.0 / den;
                    let sx = (num_x * inv_w).clamp(0.0, max_x);
                    let sy = (num_y * inv_w).clamp(0.0, max_y);

                    let x0 = (sx.floor() as usize).min(src_w - 2);
                    let y0 = (sy.floor() as usize).min(src_h - 2);
                    let fx = (sx - x0 as f64) as f32;
                    let fy = (sy - y0 as f64) as f32;

                    let w00 = (1.0 - fx) * (1.0 - fy);
                    let w10 = fx * (1.0 - fy);
                    let w01 = (1.0 - fx) * fy;
                    let w11 = fx * fy;

                    let i00 = (y0 * src_w + x0) * 3;
                    let i10 = i00 + 3;
                    let i01 = i00 + src_w * 3;
                    let i11 = i01 + 3;

                    for c in 0..3 {
                        let val = w00 * src_bytes[i00 + c] as f32
                            + w10 * src_bytes[i10 + c] as f32
                            + w01 * src_bytes[i01 + c] as f32
                            + w11 * src_bytes[i11 + c] as f32;
                        row_dst[dst_off + c] = val.round().clamp(0.0, 255.0) as u8;
                    }
                }
                num_x += h[0][0];
                num_y += h[1][0];
                den += h[2][0];
            }
        });

    RectifiedPlanePatch {
        width: homography.width_px,
        height: homography.height_px,
        rgb: out,
        homography,
    }
}
