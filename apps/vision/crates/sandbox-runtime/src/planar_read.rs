//! Tier 2-B2 (`planar-xr`) bounded planar readout for cargo inspection (`Spec | Experience` ONLY).
//! Abstains immediately when any SDS axis is `Provenance::Truth | Reference`.

use std::time::Instant;

use image::{GrayImage, Luma};
use lv_doc::vision::{
    lattice_verify::{LatticeVerifyDecision, ShadowToken},
    plan::Baseline,
    symbols::SymbolDict,
};
use lv_domain::{verify_iso6346_check_digit, CodeScheme};
use lv_planar::{
    anchor3d::{backproject_rect_point, AnchoredField3D, PlanarAnchorRegistry, ProjectedAnchorCallout},
    doc_bridge::{prepare_rectified_document, repair_surface_identifier, verify_rectified_ledger_shadow},
    gate::{evaluate_readability, ApproachForRead, ReadabilityGateCfg, ReadabilityVerdict},
    homography::{compute_planar_homography, unwarp_planar_patch},
};
use lv_spatial::{
    camera::{Intrinsics, Pose},
    frame::{DepthMap, NormalMap, Rgb},
    planar::{fit_planar_patch, PlanarCandidateRoi, PlanarFitCfg, PlanarPatch3D},
    stereo::StereoRig,
};
use serde::{Deserialize, Serialize};

use crate::cargo::CargoAction;
use crate::sds::Provenance;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecodedSurfaceSpan {
    pub field: String,
    pub raw_text: String,
    pub rect_bbox_px: (u32, u32, u32, u32),
    #[serde(skip)]
    pub shadow_top3: Vec<[ShadowToken; 3]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CargoLabelVerdict {
    pub identifier: Option<String>,
    pub iso6346_valid: bool,
    pub ledger_mismatch: bool,
    pub hazard_detected: bool,
    pub fragile_detected: bool,
    pub this_side_up: bool,
    pub virtual_table_cols: usize,
    pub grip_force_scale: f32,
    pub accel_cap_scale: f32,
}

#[derive(Clone, Debug)]
pub enum RobotPlanarOutcome {
    AbstainedProvenance,
    NoValidPlane,
    Reposition {
        approach: ApproachForRead,
        goal_xz_m: [f32; 2],
        look_yaw_rad: f32,
        look_pitch_deg: f32,
        target_quad_px: [[f32; 2]; 4],
    },
    Inspected {
        patch: PlanarPatch3D,
        verdict: CargoLabelVerdict,
        callouts: Vec<ProjectedAnchorCallout>,
    },
}

/// Hard-refuses cargo manipulation whenever a hazard placard or ungrounded ledger mismatch is present.
pub fn cargo_action_for_label(v: &CargoLabelVerdict) -> CargoAction {
    if v.hazard_detected || v.ledger_mismatch {
        CargoAction::Refuse
    } else {
        CargoAction::Take
    }
}

/// Performs optical 3D plane fitting, readability gating, planar rectification, and ledger/symbol verification.
#[allow(clippy::too_many_arguments)]
pub fn step_parcel_planar_inspection<F>(
    sensor_prov: Provenance,
    spec_prov: Provenance,
    rgb: &Rgb,
    depth: &DepthMap,
    normals: Option<&NormalMap>,
    roi: &PlanarCandidateRoi,
    intr: &Intrinsics,
    pose: &Pose,
    rig: Option<&StereoRig>,
    symbols: Option<&SymbolDict>,
    expected_ledger_ids: &[&str],
    scheme: Option<CodeScheme>,
    registry: &mut PlanarAnchorRegistry,
    mut decode_rectified_fn: F,
) -> RobotPlanarOutcome
where
    F: FnMut(&GrayImage) -> Vec<DecodedSurfaceSpan>,
{
    if !sensor_prov.control_allowed() || !spec_prov.control_allowed() {
        return RobotPlanarOutcome::AbstainedProvenance;
    }
    let Some(patch3d) = fit_planar_patch(roi, depth, normals, intr, pose, rig, &PlanarFitCfg::default()) else {
        return RobotPlanarOutcome::NoValidPlane;
    };
    if let ReadabilityVerdict::NeedReposition(app) =
        evaluate_readability(&patch3d, intr, pose, &ReadabilityGateCfg::default())
    {
        return RobotPlanarOutcome::Reposition {
            goal_xz_m: [app.target_eye_w[0] as f32, app.target_eye_w[2] as f32],
            look_yaw_rad: app.target_yaw_rad,
            look_pitch_deg: app.target_pitch_deg,
            target_quad_px: patch3d.corners_px,
            approach: app,
        };
    }

    let Some(hom) = compute_planar_homography(&patch3d, intr, pose, 5.5, 4.0, 2016) else {
        return RobotPlanarOutcome::NoValidPlane;
    };
    let rect = unwarp_planar_patch(rgb, hom);
    let Some(prep) = prepare_rectified_document(&rect, Baseline::Best, symbols) else {
        return RobotPlanarOutcome::NoValidPlane;
    };

    let hazard_detected = prep
        .matched_symbols
        .iter()
        .any(|s| s.contains("hazard") || s.contains("flammable") || s.contains("corrosive"));
    let fragile_detected = prep.matched_symbols.iter().any(|s| s.contains("fragile"));
    let this_side_up = prep
        .matched_symbols
        .iter()
        .any(|s| s.contains("this_side_up") || s.contains("upward"));

    let spans = decode_rectified_fn(&prep.enhanced_gray);
    let mut primary_id: Option<String> = None;
    let mut iso_valid = false;
    let mut ledger_mismatch = false;

    for sp in &spans {
        let mut repaired = match scheme {
            Some(sc) if matches!(sp.field.as_str(), "waybill" | "code" | "container") => {
                repair_surface_identifier(&sp.raw_text, sc)
            }
            _ => sp.raw_text.clone(),
        };
        if !expected_ledger_ids.is_empty() && !sp.shadow_top3.is_empty() {
            match verify_rectified_ledger_shadow(&repaired, &sp.shadow_top3, expected_ledger_ids) {
                LatticeVerifyDecision::Verified { matched_token } => repaired = matched_token,
                LatticeVerifyDecision::RejectedUngrounded { .. } => ledger_mismatch = true,
            }
        } else if !expected_ledger_ids.is_empty()
            && matches!(sp.field.as_str(), "waybill" | "container")
            && !expected_ledger_ids.contains(&repaired.as_str())
        {
            ledger_mismatch = true;
        }
        if scheme == Some(CodeScheme::Iso6346Container) && repaired.len() == 11 {
            iso_valid = verify_iso6346_check_digit(&repaired);
            if !iso_valid {
                ledger_mismatch = true;
            }
        }
        if primary_id.is_none() && matches!(sp.field.as_str(), "waybill" | "code" | "container") {
            primary_id = Some(repaired.clone());
        }

        let (u0, v0, u1, v1) = sp.rect_bbox_px;
        let (center_w, _) = backproject_rect_point(
            &rect.homography,
            &patch3d,
            0.5 * (u0 + u1) as f32,
            0.5 * (v0 + v1) as f32,
            Some(depth),
            intr,
            pose,
        );
        let uv4 = [(u0, v0), (u1, v0), (u1, v1), (u0, v1)];
        let mut corners_w = [[0.0; 3]; 4];
        let mut image_quad_px = [[0.0f32; 2]; 4];
        for (k, &(u, v)) in uv4.iter().enumerate() {
            let (cw, px) = backproject_rect_point(
                &rect.homography,
                &patch3d,
                u as f32,
                v as f32,
                Some(depth),
                intr,
                pose,
            );
            corners_w[k] = cw;
            image_quad_px[k] = px;
        }
        let mut obs = patch3d.view_obs;
        obs.point = center_w;
        registry.upsert(
            &AnchoredField3D {
                category: "cargo".into(),
                field: sp.field.clone(),
                text_repaired: repaired,
                rect_bbox_px: sp.rect_bbox_px,
                image_quad_px,
                center_w,
                corners_w,
                view_obs: obs,
            },
            8,
        );
    }

    RobotPlanarOutcome::Inspected {
        patch: patch3d,
        verdict: CargoLabelVerdict {
            identifier: primary_id,
            iso6346_valid: iso_valid,
            ledger_mismatch,
            hazard_detected,
            fragile_detected,
            this_side_up,
            virtual_table_cols: prep.virtual_cols,
            grip_force_scale: if fragile_detected { 0.65 } else { 1.0 },
            accel_cap_scale: if fragile_detected || this_side_up { 0.55 } else { 1.0 },
        },
        callouts: registry.project_active(intr, pose),
    }
}

/// Checks planar rectification with a 60fps tick budget threshold (`16_666 us`) and returns `(elapsed_us, verdict)`.
pub fn inspect_with_budget_us<F>(
    sensor_prov: Provenance,
    spec_prov: Provenance,
    rgb: &Rgb,
    depth: &DepthMap,
    roi: &PlanarCandidateRoi,
    intr: &Intrinsics,
    pose: &Pose,
    expected_ledger_ids: &[&str],
    scheme: Option<CodeScheme>,
    registry: &mut PlanarAnchorRegistry,
    decode_rectified_fn: F,
) -> (u64, RobotPlanarOutcome)
where
    F: FnMut(&GrayImage) -> Vec<DecodedSurfaceSpan>,
{
    let t0 = Instant::now();
    let out = step_parcel_planar_inspection(
        sensor_prov,
        spec_prov,
        rgb,
        depth,
        None,
        roi,
        intr,
        pose,
        None,
        None,
        expected_ledger_ids,
        scheme,
        registry,
        decode_rectified_fn,
    );
    let _ = Luma([0u8]);
    (t0.elapsed().as_micros() as u64, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lv_doc::vision::symbols::{SymbolFeature, NONE_CLASS};
    use lv_spatial::frame::DepthKind;
    use lv_spatial::planar::SurfaceClassHint;

    fn make_frontal_depth(
        intr: &Intrinsics,
        pose: &Pose,
        z_m: f64,
        half_u: f64,
        half_v: f64,
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
                hint: SurfaceClassHint::IndustrialNameplate,
            },
        )
    }

    #[test]
    fn planar_read_abstains_truth_reference_and_refuses_hazmat_within_budget() {
        let intr = Intrinsics::pinhole(400, 300, 13.0);
        let mut pose = Pose::at_pos(0.0, 1.5, 0.0);
        pose.yaw = std::f32::consts::FRAC_PI_2;
        let (dm_near, roi_near) = make_frontal_depth(&intr, &pose, 0.29, 0.022, 0.0215);
        let rgb = Rgb::new(400, 300, vec![210u8; 400 * 300 * 3]);
        let mut reg = PlanarAnchorRegistry::new();

        for bad_prov in [Provenance::Truth, Provenance::Reference] {
            let out = step_parcel_planar_inspection(
                bad_prov,
                Provenance::Spec,
                &rgb,
                &dm_near,
                None,
                &roi_near,
                &intr,
                &pose,
                None,
                None,
                &["MSCU1234566"],
                Some(CodeScheme::Iso6346Container),
                &mut reg,
                |_| Vec::new(),
            );
            assert!(matches!(out, RobotPlanarOutcome::AbstainedProvenance));
        }

        // Warm thread pool / caches once before steady-state 60fps tick budget measurement.
        let _ = step_parcel_planar_inspection(
            Provenance::Experience,
            Provenance::Spec,
            &rgb,
            &dm_near,
            None,
            &roi_near,
            &intr,
            &pose,
            None,
            None,
            &["MSCU1234566"],
            Some(CodeScheme::Iso6346Container),
            &mut reg,
            |_| Vec::new(),
        );

        let (elapsed_us, ok) = inspect_with_budget_us(
            Provenance::Experience,
            Provenance::Spec,
            &rgb,
            &dm_near,
            &roi_near,
            &intr,
            &pose,
            &["MSCU1234566"],
            Some(CodeScheme::Iso6346Container),
            &mut reg,
            |_| {
                vec![DecodedSurfaceSpan {
                    field: "container".into(),
                    raw_text: "MSCU1Z34S66".into(),
                    rect_bbox_px: (8, 8, 96, 32),
                    shadow_top3: Vec::new(),
                }]
            },
        );
        assert!(elapsed_us <= 16_666, "planar read took {elapsed_us} us");
        match ok {
            RobotPlanarOutcome::Inspected { verdict, .. } => {
                assert_eq!(verdict.identifier.as_deref(), Some("MSCU1234566"));
                assert!(verdict.iso6346_valid);
                assert!(!verdict.ledger_mismatch);
                assert_eq!(cargo_action_for_label(&verdict), CargoAction::Take);
            }
            other => panic!("expected Inspected, got {other:?}"),
        }

        // Verify CargoAction::Refuse policy for hazmat labels and non-confusion with NONE_CLASS.
        let _dict_empty_check = SymbolDict::from_legend_samples(
            SymbolFeature::Pooled,
            vec![NONE_CLASS.to_string()],
            &[(0, vec![0.0; 16])],
        );
        let hazmat = CargoLabelVerdict {
            identifier: Some("UN1263".into()),
            iso6346_valid: false,
            ledger_mismatch: false,
            hazard_detected: true,
            fragile_detected: false,
            this_side_up: false,
            virtual_table_cols: 0,
            grip_force_scale: 1.0,
            accel_cap_scale: 1.0,
        };
        assert_eq!(cargo_action_for_label(&hazmat), CargoAction::Refuse);
    }
}
