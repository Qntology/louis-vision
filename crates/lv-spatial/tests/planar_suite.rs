//! Gate P1 integration tests for `lv-spatial::planar` and `Intrinsics` unprojection/projection.

use lv_spatial::camera::{Intrinsics, Pose};
use lv_spatial::frame::{DepthKind, DepthMap};
use lv_spatial::planar::{
    fit_planar_patch, PhysicalFormFactor, PlanarCandidateRoi, PlanarFitCfg, SurfaceClassHint,
};

fn synth_plane_depth(
    intr: &Intrinsics,
    pose: &Pose,
    center_w: [f64; 3],
    normal_w: [f64; 3],
    u_w: [f64; 3],
    v_w: [f64; 3],
    half_u_m: f64,
    half_v_m: f64,
    glare_every: usize,
    outlier_every: usize,
) -> (DepthMap, PlanarCandidateRoi) {
    let w = intr.width;
    let h = intr.height;
    let mut data = vec![f32::NAN; (w * h) as usize];
    let eye = [pose.x as f64, pose.eye_m as f64, pose.z as f64];
    let mut min_px = w as f32;
    let mut max_px = 0.0f32;
    let mut min_py = h as f32;
    let mut max_py = 0.0f32;
    let mut count = 0usize;

    for py in 0..h {
        for px in 0..w {
            let r = intr.ray(pose.yaw, px as f32 + 0.5, py as f32 + 0.5);
            let dir = [r[0] as f64, r[1] as f64, r[2] as f64];
            let denom = dir[0] * normal_w[0] + dir[1] * normal_w[1] + dir[2] * normal_w[2];
            if denom.abs() < 1e-6 {
                continue;
            }
            let oc = [center_w[0] - eye[0], center_w[1] - eye[1], center_w[2] - eye[2]];
            let t = (oc[0] * normal_w[0] + oc[1] * normal_w[1] + oc[2] * normal_w[2]) / denom;
            if t <= 0.1 {
                continue;
            }
            let rel = [
                eye[0] + t * dir[0] - center_w[0],
                eye[1] + t * dir[1] - center_w[1],
                eye[2] + t * dir[2] - center_w[2],
            ];
            let u = rel[0] * u_w[0] + rel[1] * u_w[1] + rel[2] * u_w[2];
            let v = rel[0] * v_w[0] + rel[1] * v_w[1] + rel[2] * v_w[2];
            if u.abs() <= half_u_m && v.abs() <= half_v_m {
                min_px = min_px.min(px as f32);
                max_px = max_px.max(px as f32 + 1.0);
                min_py = min_py.min(py as f32);
                max_py = max_py.max(py as f32 + 1.0);
                count += 1;
                if glare_every > 0 && (count % glare_every) == 0 {
                    data[(py * w + px) as usize] = f32::NAN;
                } else if outlier_every > 0 && (count % outlier_every) == 0 {
                    data[(py * w + px) as usize] = 3.5;
                } else {
                    let fwd = intr.camera_axes(pose.yaw).0;
                    let z_cam = dir[0] * fwd[0] as f64 + dir[1] * fwd[1] as f64 + dir[2] * fwd[2] as f64;
                    data[(py * w + px) as usize] = (t * z_cam) as f32;
                }
            }
        }
    }
    let dm = DepthMap {
        width: w,
        height: h,
        kind: DepthKind::Metric,
        data,
    };
    let roi = PlanarCandidateRoi {
        x0: (min_px.floor().max(0.0)) as u32,
        y0: (min_py.floor().max(0.0)) as u32,
        x1: (max_px.ceil() as u32).min(w),
        y1: (max_py.ceil() as u32).min(h),
        hint: SurfaceClassHint::PaperDocument,
    };
    (dm, roi)
}

#[test]
fn gate_p1_roundtrip_and_slanted_plane_recovery() {
    let intr = Intrinsics::pinhole(640, 480, 62.0);
    let mut pose = Pose::at_pos(0.2, 1.5, -0.3);
    pose.yaw = 0.65;

    for i in 0..100 {
        let px = 40.0 + (i % 10) as f32 * 52.0;
        let py = 30.0 + (i / 10) as f32 * 40.0;
        let d = 0.6 + (i % 5) as f32 * 0.25;
        let wpt = intr.unproject_optical(&pose, px, py, d);
        let (rx, ry, rz) = intr.project_world_unclamped(&pose, wpt).expect("projected");
        assert!((px - rx).abs() < 1e-3 && (py - ry).abs() < 1e-3);
        assert!((d - rz).abs() < 1e-4);
    }

    pose.yaw = std::f32::consts::FRAC_PI_2;
    // 35 deg slanted A4 document (210 x 297 mm) with outliers + glare holes
    let tilt = 35.0f64.to_radians();
    let n_w = [0.0, tilt.sin(), -tilt.cos()];
    let u_w = [1.0, 0.0, 0.0];
    let v_w = [0.0, tilt.cos(), tilt.sin()];
    let cam_fwd = intr.camera_axes(pose.yaw).0;
    let center = [
        pose.x as f64 + 0.72 * cam_fwd[0] as f64,
        pose.eye_m as f64 + 0.72 * cam_fwd[1] as f64,
        pose.z as f64 + 0.72 * cam_fwd[2] as f64,
    ];
    let (mut dm, roi) = synth_plane_depth(&intr, &pose, center, n_w, u_w, v_w, 0.105, 0.1485, 3, 4);
    let patch = fit_planar_patch(&roi, &dm, None, &intr, &pose, None, &PlanarFitCfg::default())
        .expect("slanted A4 plane");

    let cos_ang = (patch.normal_w[0] * n_w[0] + patch.normal_w[1] * n_w[1] + patch.normal_w[2] * n_w[2]).abs().min(1.0);
    let deg_err = cos_ang.acos().to_degrees();
    assert!(deg_err <= 1.5, "normal error {deg_err:.2} deg");
    assert!(
        (patch.width_mm - 210.0).abs() / 210.0 <= 0.030,
        "width_mm={} expected ~210", patch.width_mm
    );
    assert!(
        (patch.height_mm - 297.0).abs() / 297.0 <= 0.030,
        "height_mm={} (width_mm={}, inliers={}, hole_frac={}) expected ~297",
        patch.height_mm, patch.width_mm, patch.inliers, patch.depth_hole_frac
    );
    assert!(patch.plane_rmse_m <= 0.002, "rmse={}", patch.plane_rmse_m);
    assert_eq!(patch.form_factor, PhysicalFormFactor::IsoA4);

    // Relative depth MUST be rejected
    dm.kind = DepthKind::Relative;
    assert!(fit_planar_patch(&roi, &dm, None, &intr, &pose, None, &PlanarFitCfg::default()).is_none());
}
