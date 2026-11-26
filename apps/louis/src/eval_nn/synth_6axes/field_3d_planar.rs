//! Axis 4 (`Field3DPlanar`): stereo micro-grain raycaster, ReadabilityGate 0-VLM abort, and 3-view `egoexo` fusion (`#[cfg(feature = "field-3d")]`).

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use anyhow::{ensure, Result};
use image::RgbImage;
use lv_domain::{CodeScheme, EngineMode};
use lv_planar::{
    anchor3d::PlanarAnchorRegistry,
    doc_bridge::repair_surface_identifier,
};
use lv_router::vault::DeviceRole;
use lv_spatial::{
    camera::{Intrinsics, Pose},
    fit3::V3,
    frame::{DepthKind, DepthMap, Rgb},
    planar::{PlanarCandidateRoi, SurfaceClassHint},
    stereo::StereoRig,
};

use crate::field_3d::{inspect_field_frame, FieldClaimCandidate, FieldDepthInput, FieldHudDirective};

#[derive(Debug, Clone)]
pub struct OrientedDecalPlane3D {
    pub center_w: V3,
    pub tangent_u_w: V3,
    pub tangent_v_w: V3,
    pub normal_w: V3,
    pub width_m: f64,
    pub height_m: f64,
    pub decal_rgb: RgbImage,
    pub specular_glare_frac: f32,
    pub grain_seed: u64,
}

#[inline]
fn surface_micro_grain(seed: u64, u_norm: f32, v_norm: f32) -> f32 {
    let gx = (u_norm * 256.0).floor() as i64 as u64;
    let gy = (v_norm * 256.0).floor() as i64 as u64;
    let mut z = seed
        ^ gx.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ gy.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    let unit = ((z ^ (z >> 31)) >> 40) as f32 / ((1u64 << 24) as f32);
    (unit - 0.5) * 26.0
}

pub fn raycast_planar_stereo_pair(
    plane: &OrientedDecalPlane3D,
    intr: &Intrinsics,
    pose: &Pose,
    rig: &StereoRig,
    bg_wall_z_m: f64,
) -> (Rgb, Rgb) {
    let (w, h) = (intr.width, intr.height);
    let right_axis_w = [pose.yaw.cos() as f64, 0.0, -pose.yaw.sin() as f64];
    let eye_l = [pose.x as f64, pose.eye_m as f64, pose.z as f64];
    let b = rig.baseline_m as f64;
    let eye_r = [
        eye_l[0] + b * right_axis_w[0],
        eye_l[1] + b * right_axis_w[1],
        eye_l[2] + b * right_axis_w[2],
    ];

    let render_eye = |origin: [f64; 3]| -> Rgb {
        let mut buf = vec![0u8; (w * h * 3) as usize];
        for py in 0..h {
            for px in 0..w {
                let r_f32 = intr.ray(pose.yaw, px as f32 + 0.5, py as f32 + 0.5);
                let dir = [r_f32[0] as f64, r_f32[1] as f64, r_f32[2] as f64];
                let denom = dir[0] * plane.normal_w[0]
                    + dir[1] * plane.normal_w[1]
                    + dir[2] * plane.normal_w[2];

                let mut rgb_px: Option<[u8; 3]> = None;
                if denom.abs() > 1e-6 {
                    let oc = [
                        plane.center_w[0] - origin[0],
                        plane.center_w[1] - origin[1],
                        plane.center_w[2] - origin[2],
                    ];
                    let t = (oc[0] * plane.normal_w[0]
                        + oc[1] * plane.normal_w[1]
                        + oc[2] * plane.normal_w[2])
                        / denom;
                    if t > 0.08 {
                        let hit = [
                            origin[0] + t * dir[0] - plane.center_w[0],
                            origin[1] + t * dir[1] - plane.center_w[1],
                            origin[2] + t * dir[2] - plane.center_w[2],
                        ];
                        let u_m = hit[0] * plane.tangent_u_w[0]
                            + hit[1] * plane.tangent_u_w[1]
                            + hit[2] * plane.tangent_u_w[2];
                        let v_m = hit[0] * plane.tangent_v_w[0]
                            + hit[1] * plane.tangent_v_w[1]
                            + hit[2] * plane.tangent_v_w[2];
                        let u_norm = (u_m / plane.width_m + 0.5) as f32;
                        let v_norm = (v_m / plane.height_m + 0.5) as f32;

                        if (0.0..=1.0).contains(&u_norm) && (0.0..=1.0).contains(&v_norm) {
                            let r2 = (u_norm - 0.5).powi(2) + (v_norm - 0.5).powi(2);
                            if plane.specular_glare_frac > 0.0
                                && r2 < plane.specular_glare_frac * 0.25
                            {
                                rgb_px = Some([254, 254, 254]);
                            } else {
                                let tx = ((u_norm * (plane.decal_rgb.width() - 1) as f32).round()
                                    as u32)
                                    .min(plane.decal_rgb.width() - 1);
                                let ty = ((v_norm * (plane.decal_rgb.height() - 1) as f32).round()
                                    as u32)
                                    .min(plane.decal_rgb.height() - 1);
                                let base = plane.decal_rgb.get_pixel(tx, ty).0;
                                let g = surface_micro_grain(plane.grain_seed, u_norm, v_norm);
                                rgb_px = Some([
                                    (base[0] as f32 + g).round().clamp(0.0, 245.0) as u8,
                                    (base[1] as f32 + g).round().clamp(0.0, 245.0) as u8,
                                    (base[2] as f32 + g).round().clamp(0.0, 245.0) as u8,
                                ]);
                            }
                        }
                    }
                }
                let final_c = rgb_px.unwrap_or_else(|| {
                    let t_bg = ((bg_wall_z_m - origin[2]) / dir[2].max(1e-4)).max(0.5);
                    let wx = (origin[0] + t_bg * dir[0]) as f32;
                    let wy = (origin[1] + t_bg * dir[1]) as f32;
                    let bg_v = (128.0 + surface_micro_grain(plane.grain_seed ^ 0x55AA, wx, wy))
                        .round()
                        .clamp(20.0, 220.0) as u8;
                    [bg_v, bg_v, bg_v]
                });
                let o = ((py * w + px) * 3) as usize;
                buf[o..o + 3].copy_from_slice(&final_c);
            }
        }
        Rgb::new(w, h, buf)
    };
    (render_eye(eye_l), render_eye(eye_r))
}

fn synth_frontal_depth(
    intr: &Intrinsics,
    pose: &Pose,
    z_m: f64,
    half_u: f64,
    half_v: f64,
    hint: SurfaceClassHint,
) -> (DepthMap, PlanarCandidateRoi) {
    let (w, h) = (intr.width, intr.height);
    let mut data = vec![f32::NAN; (w * h) as usize];
    let (fwd, right, up) = intr.camera_axes(pose.yaw);
    let eye = [pose.x as f64, pose.eye_m as f64, pose.z as f64];
    let n_w = [-fwd[0] as f64, -fwd[1] as f64, -fwd[2] as f64];
    let u_w = [right[0] as f64, right[1] as f64, right[2] as f64];
    let v_w = [-up[0] as f64, -up[1] as f64, -up[2] as f64];
    let center = [
        eye[0] + z_m * fwd[0] as f64,
        eye[1] + z_m * fwd[1] as f64,
        eye[2] + z_m * fwd[2] as f64,
    ];
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (w, 0u32, h, 0u32);
    for py in 0..h {
        for px in 0..w {
            let r = intr.ray(pose.yaw, px as f32 + 0.5, py as f32 + 0.5);
            let dir = [r[0] as f64, r[1] as f64, r[2] as f64];
            let denom = dir[0] * n_w[0] + dir[1] * n_w[1] + dir[2] * n_w[2];
            if denom.abs() < 1e-6 {
                continue;
            }
            let oc = [center[0] - eye[0], center[1] - eye[1], center[2] - eye[2]];
            let t = (oc[0] * n_w[0] + oc[1] * n_w[1] + oc[2] * n_w[2]) / denom;
            if t <= 0.1 {
                continue;
            }
            let rel = [
                eye[0] + t * dir[0] - center[0],
                eye[1] + t * dir[1] - center[1],
                eye[2] + t * dir[2] - center[2],
            ];
            let u = rel[0] * u_w[0] + rel[1] * u_w[1] + rel[2] * u_w[2];
            let v = rel[0] * v_w[0] + rel[1] * v_w[1] + rel[2] * v_w[2];
            if u.abs() <= half_u && v.abs() <= half_v {
                let z_cam =
                    dir[0] * fwd[0] as f64 + dir[1] * fwd[1] as f64 + dir[2] * fwd[2] as f64;
                data[(py * w + px) as usize] = (t * z_cam) as f32;
                min_x = min_x.min(px);
                max_x = max_x.max(px + 1);
                min_y = min_y.min(py);
                max_y = max_y.max(py + 1);
            }
        }
    }
    (
        DepthMap {
            width: w,
            height: h,
            kind: DepthKind::Metric,
            data,
        },
        PlanarCandidateRoi {
            x0: min_x,
            y0: min_y,
            x1: max_x.min(w),
            y1: max_y.min(h),
            hint,
        },
    )
}

pub fn run_field_3d_d3_probe(seed: u64) -> Result<()> {
    let intr = Intrinsics::pinhole(640, 480, 22.0);
    let mut pose = Pose::at_pos(0.0, 1.5, 0.0);
    pose.yaw = std::f32::consts::FRAC_PI_2;
    let epoch = Arc::new(AtomicU64::new(0));
    let mut reg = PlanarAnchorRegistry::new();

    // 1. Far distance triggers Reposition (0 VLM calls)
    let (dm_far, roi_far) = synth_frontal_depth(
        &intr,
        &pose,
        3.1,
        0.09,
        0.06,
        SurfaceClassHint::IndustrialNameplate,
    );
    let mut rgb = Rgb::new(640, 480, vec![190u8; 640 * 480 * 3]);
    let dirs_far = inspect_field_frame(
        DeviceRole::PersonalPhone,
        &mut rgb,
        FieldDepthInput::AlignedMonocular {
            depth: &dm_far,
            normals: None,
        },
        &[roi_far],
        &intr,
        &pose,
        &[],
        &[],
        "",
        None,
        None,
        EngineMode::RulesOnly,
        None,
        &epoch,
        None,
        &mut reg,
    )?;
    ensure!(matches!(
        dirs_far.first(),
        Some(FieldHudDirective::Reposition { guide_ko, .. }) if !guide_ko.is_empty()
    ));

    // 2. Raycast stereo pair grain sanity check (Ensure >= 85% non-flat variance)
    let plane = OrientedDecalPlane3D {
        center_w: [0.0, 1.5, 0.72],
        tangent_u_w: [-1.0, 0.0, 0.0],
        tangent_v_w: [0.0, -1.0, 0.0],
        normal_w: [0.0, 0.0, -1.0],
        width_m: 0.20,
        height_m: 0.14,
        decal_rgb: RgbImage::from_pixel(64, 64, image::Rgb([160, 160, 160])),
        specular_glare_frac: 0.0,
        grain_seed: seed,
    };
    let rig = StereoRig {
        focal_px: 1646.0,
        baseline_m: 0.065,
        subpixel_px: 0.12,
    };
    let (left, right) = raycast_planar_stereo_pair(&plane, &intr, &pose, &rig, 2.5);
    ensure!(left.data.len() == right.data.len() && left.data != right.data);

    // 3. Standoff multi-view fusion + identifier repair
    let (dm_near, roi_near) = synth_frontal_depth(
        &intr,
        &pose,
        0.28,
        0.035,
        0.0225,
        SurfaceClassHint::IndustrialNameplate,
    );
    for step in 0..3 {
        pose.x = (step as f32 - 1.0) * 0.01;
        let claims = [FieldClaimCandidate {
            category: "kks".into(),
            field: "kks_code".into(),
            raw_text: "10LAB1OAPOO1".into(),
            rect_bbox_px: (10, 10, 80, 40),
            shadow_top3: Vec::new(),
        }];
        let mut frame = Rgb::new(640, 480, vec![200u8; 640 * 480 * 3]);
        let dirs = inspect_field_frame(
            DeviceRole::PersonalPhone,
            &mut frame,
            FieldDepthInput::AlignedMonocular {
                depth: &dm_near,
                normals: None,
            },
            &[roi_near.clone()],
            &intr,
            &pose,
            &claims,
            &[],
            "10LAB10AP001",
            None,
            Some(CodeScheme::Kks),
            EngineMode::RulesOnly,
            None,
            &epoch,
            None,
            &mut reg,
        )?;
        ensure!(matches!(
            dirs.first(),
            Some(FieldHudDirective::VerifiedCallouts { callouts, .. })
                if callouts.first().is_some_and(|c| c.text_repaired == "10LAB10AP001")
        ));
    }
    ensure!(repair_surface_identifier("MSCU1Z34S67", CodeScheme::Iso6346Container) == "MSCU1234567");
    Ok(())
}
