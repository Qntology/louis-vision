//! Gate P2 integration tests for `lv-planar` (closed-form H, lv-doc bridge, ApproachForRead, 3D quad zeroize).

use std::collections::HashMap;
#[cfg(feature = "screen")]
use std::sync::atomic::AtomicU64;

use lv_doc::vision::lattice_verify::{LatticeVerifyDecision, ShadowToken};
use lv_doc::vision::plan::Baseline;
use lv_doc::vision::table_grid_reverse::VIRTUAL_GRIDLINE_LUMA;
use lv_domain::CodeScheme;
use lv_planar::{
    doc_bridge::{prepare_rectified_document, repair_surface_identifier, verify_rectified_ledger_shadow},
    gate::{evaluate_readability, ReadabilityGateCfg, ReadabilityVerdict},
    homography::{compute_planar_homography, unwarp_planar_patch},
};
#[cfg(feature = "screen")]
use lv_planar::screen_bridge::{
    inspect_and_guard_3d_screen, PlanarScreenGuardCfg, PlanarScreenOutcome, ScreenRedactReason,
};
use lv_spatial::camera::{Intrinsics, Pose};
use lv_spatial::egoexo::ViewObs;
use lv_spatial::frame::Rgb;
use lv_spatial::planar::{PhysicalFormFactor, PlanarPatch3D, SurfaceClassHint};
use smallvec::SmallVec;

fn make_patch(center_w: [f64; 3], px_per_mm: f32, hint: SurfaceClassHint, ff: PhysicalFormFactor) -> PlanarPatch3D {
    PlanarPatch3D {
        center_w,
        normal_w: [0.0, 0.0, -1.0],
        tangent_u_w: [-1.0, 0.0, 0.0],
        tangent_v_w: [0.0, -1.0, 0.0],
        width_mm: 120.0,
        height_mm: 80.0,
        form_factor: ff,
        hint,
        corners_w: [
            [center_w[0] - 0.06, center_w[1] - 0.04, center_w[2]],
            [center_w[0] + 0.06, center_w[1] - 0.04, center_w[2]],
            [center_w[0] + 0.06, center_w[1] + 0.04, center_w[2]],
            [center_w[0] - 0.06, center_w[1] + 0.04, center_w[2]],
        ],
        corners_px: [[120.0, 120.0], [520.0, 120.0], [520.0, 360.0], [120.0, 360.0]],
        effective_px_per_mm_u: px_per_mm,
        effective_px_per_mm_v: px_per_mm,
        view_cos: 0.99,
        depth_hole_frac: 0.1,
        plane_rmse_m: 0.001,
        sigma_z_m: 0.005,
        inliers: 100,
        view_obs: ViewObs {
            origin: [0.0, 1.5, 0.0],
            point: center_w,
            sigma_z: 0.005,
            sigma_theta: 0.003,
            weight: 1.0,
        },
    }
}

#[test]
fn gate_p2_homography_doc_bridge_and_fail_closed_screen_zeroize() {
    let intr = Intrinsics::pinhole(640, 480, 62.0);
    let mut pose = Pose::at_pos(0.0, 1.5, 0.0);
    pose.yaw = std::f32::consts::FRAC_PI_2;

    let plaque = make_patch([0.0, 1.5, 0.72], 6.0, SurfaceClassHint::IndustrialNameplate, PhysicalFormFactor::IndustrialPlaque);
    let h = compute_planar_homography(&plaque, &intr, &pose, 5.5, 4.0, 2016).expect("homography");
    for (u, v) in [(0.0, 0.0), (100.0, 50.0), (h.width_px as f64 - 1.0, h.height_px as f64 - 1.0)] {
        let (px, py, _) = h.rect_to_img(u, v).unwrap();
        let (ru, rv) = h.img_to_rect(px, py).unwrap();
        assert!((u - ru).abs() < 1e-4 && (v - rv).abs() < 1e-4);
    }

    assert_eq!(repair_surface_identifier("10LAB1OAPOO1", CodeScheme::Kks), "10LAB10AP001");
    assert_eq!(repair_surface_identifier("KR1SEOO2RK04", CodeScheme::Dcim), "KR1SEO02RK04");
    assert_eq!(repair_surface_identifier("MSCU1Z34S67", CodeScheme::Iso6346Container), "MSCU1234567");

    let shadow_slots = [[
        ShadowToken { bytes: SmallVec::from_slice(b"10LAB10AP001"), prob: 0.42 },
        ShadowToken { bytes: SmallVec::from_slice(b"10LAB10AP002"), prob: 0.05 },
        ShadowToken { bytes: SmallVec::from_slice(b"10LAB20AP001"), prob: 0.03 },
    ]];
    assert_eq!(
        verify_rectified_ledger_shadow("10LAB1OAPOO1", &shadow_slots, &["10LAB10AP001"]),
        LatticeVerifyDecision::Verified { matched_token: "10LAB10AP001".into() }
    );

    // Distant plaque fails ReadabilityGate without VLM calls
    let far_plaque = make_patch([0.0, 1.5, 3.1], 1.55, SurfaceClassHint::IndustrialNameplate, PhysicalFormFactor::IndustrialPlaque);
    assert!(matches!(evaluate_readability(&far_plaque, &intr, &pose, &ReadabilityGateCfg::default()), ReadabilityVerdict::NeedReposition(_)));

    // 3D screen fail-closed without tower zeroes perspective quad completely
    #[allow(unused_mut)]
    let mut rgb = Rgb::new(640, 480, vec![123u8; 640 * 480 * 3]);
    let screen_patch = make_patch([0.0, 1.5, 0.85], 4.0, SurfaceClassHint::DisplayScreen, PhysicalFormFactor::Display16x9);
    let h_scr = compute_planar_homography(&screen_patch, &intr, &pose, 4.0, 2.0, 2016).unwrap();
    #[allow(unused_mut)]
    let mut rect = unwarp_planar_patch(&rgb, h_scr);
    let doc_prep = prepare_rectified_document(&rect, Baseline::Best, None).unwrap();
    let _ = doc_prep.has_protected_titleblock;
    let _ = VIRTUAL_GRIDLINE_LUMA;

    #[cfg(feature = "screen")]
    {
        let epoch = AtomicU64::new(0);
        let res = inspect_and_guard_3d_screen(&mut rgb, &mut rect, &screen_patch, None, &epoch, 0, &PlanarScreenGuardCfg::default()).unwrap();
        match res {
            PlanarScreenOutcome::PrivacyRedacted { zeroized_pixels, reason: ScreenRedactReason::FailClosedNoTower, .. } => {
                assert!(zeroized_pixels > 1000);
            }
            other => panic!("expected fail-closed redaction, got {other:?}"),
        }
        // Exterior corner pixel untouched
        assert_eq!(rgb.px(0, 0), [123, 123, 123]);
        // Interior center pixel zeroed
        assert_eq!(rgb.px(320, 240), [0, 0, 0]);
    }
    let _ = HashMap::<String, f32>::new();
}
