//! Field-3D planar inspection & fail-closed privacy pipeline (`#[cfg(feature = "field-3d")]`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{bail, ensure, Result};
use lv_agent::factcheck::draft_or_fallback;
use lv_core::siglip2::VisionTower;
use lv_doc::vision::{
    lattice_verify::{LatticeVerifyDecision, ShadowToken},
    plan::Baseline,
};
use lv_domain::{
    evaluate_pack, verify_iso6346_check_digit, CodeScheme, DocsDomainPack, EngineMode,
    PackCheckInput, PackCheckResult,
};
use lv_planar::{
    anchor3d::{backproject_rect_point, AnchoredField3D, PlanarAnchorRegistry},
    doc_bridge::{
        prepare_rectified_document, repair_surface_identifier, verify_rectified_ledger_shadow,
    },
    gate::{
        evaluate_readability, ApproachForRead, ReadabilityBlockReason, ReadabilityGateCfg,
        ReadabilityVerdict,
    },
    homography::{compute_planar_homography, unwarp_planar_patch},
    screen_bridge::{inspect_and_guard_3d_screen, PlanarScreenGuardCfg, PlanarScreenOutcome},
};
use lv_router::vault::{DeviceRole, VaultError};
use lv_screen::AnchorBank;
use lv_spatial::{
    camera::{Intrinsics, Pose},
    frame::{DepthKind, DepthMap, NormalMap, Rgb},
    planar::{
        fit_planar_patch, PhysicalFormFactor, PlanarCandidateRoi, PlanarFitCfg, SurfaceClassHint,
    },
    stereo::{sgbm, SgbmCfg, StereoRig},
};
use serde::{Deserialize, Serialize};

use crate::overlay::OverlayController;
use crate::watcher::{IngestResult, StagedLouisPipeline};

pub enum FieldDepthInput<'a> {
    StereoPair {
        left: &'a Rgb,
        right: &'a Rgb,
        rig: &'a StereoRig,
        cfg: SgbmCfg,
    },
    AlignedMonocular {
        depth: &'a DepthMap,
        normals: Option<&'a NormalMap>,
    },
}

pub struct FieldScreenGuardCtx<'a> {
    pub models: Option<(&'a VisionTower, &'a AnchorBank)>,
    pub cfg: PlanarScreenGuardCfg,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FieldClaimCandidate {
    pub category: String,
    pub field: String,
    pub raw_text: String,
    pub rect_bbox_px: (u32, u32, u32, u32),
    #[serde(skip)]
    pub shadow_top3: Vec<[ShadowToken; 3]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FieldHudDirective {
    Reposition {
        target_quad_px: [[f32; 2]; 4],
        approach: ApproachForRead,
        guide_ko: String,
    },
    PrivacyRedacted {
        surface_id: u32,
        zeroized_pixels: usize,
        reason: String,
    },
    VerifiedCallouts {
        surface_id: u32,
        form_factor: String,
        virtual_table_cols: usize,
        has_protected_titleblock: bool,
        pack_result: Option<PackCheckResult>,
        factcheck_grounded: bool,
        callouts: Vec<AnchoredField3D>,
    },
}

fn format_hud_guide_ko(app: &ApproachForRead) -> String {
    let dist_m =
        (app.delta_move_w[0].powi(2) + app.delta_move_w[1].powi(2) + app.delta_move_w[2].powi(2))
            .sqrt();
    match app.reason {
        ReadabilityBlockReason::LowEffectiveDpi => format!(
            "해상도 부족 ({:.1} < {:.1} px/mm) - 표면 정면으로 {:.2}m 접근하세요",
            app.observed_px_per_mm, app.required_px_per_mm, dist_m
        ),
        ReadabilityBlockReason::SteepViewingAngle => format!(
            "촬영 각도가 너무 비스듬합니다 - 표면 법선 정면 축으로 {:.2}m 이동해 정렬하세요",
            dist_m
        ),
        ReadabilityBlockReason::SpecularGlareHoles => format!(
            "유광 빛반사 감지 - 반사각 회피(10 deg)를 위해 측면 포함 {:.2}m 이동하세요",
            dist_m
        ),
        ReadabilityBlockReason::ClippedByFrameEdge => {
            "문서/명판 모서리가 화면 밖으로 잘렸습니다 - 전체가 들어오도록 시선을 조정하세요"
                .into()
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn inspect_field_frame(
    role: DeviceRole,
    rgb: &mut Rgb,
    depth_in: FieldDepthInput<'_>,
    rois: &[PlanarCandidateRoi],
    intr: &Intrinsics,
    pose: &Pose,
    claims: &[FieldClaimCandidate],
    ledger_candidates: &[&str],
    source_evidence_text: &str,
    pack_input: Option<(DocsDomainPack, i64, i64)>,
    scheme: Option<CodeScheme>,
    mode: EngineMode,
    screen_guard: Option<&FieldScreenGuardCtx<'_>>,
    privacy_epoch: &Arc<AtomicU64>,
    overlay: Option<&mut OverlayController>,
    registry: &mut PlanarAnchorRegistry,
) -> Result<Vec<FieldHudDirective>> {
    if role == DeviceRole::CounterKiosk {
        bail!(VaultError::KioskForbidden);
    }

    let owned_stereo;
    let (depth, normals, rig): (&DepthMap, Option<&NormalMap>, Option<&StereoRig>) = match depth_in
    {
        FieldDepthInput::StereoPair {
            left,
            right,
            rig,
            ref cfg,
        } => {
            owned_stereo = sgbm(left, right, rig, cfg, None)?;
            (&owned_stereo.depth, None, Some(rig))
        }
        FieldDepthInput::AlignedMonocular { depth, normals } => {
            ensure!(
                depth.kind == DepthKind::Metric,
                "monocular depth must be DepthKind::Metric"
            );
            (depth, normals, None)
        }
    };

    let fit_cfg = PlanarFitCfg::default();
    let gate_cfg = ReadabilityGateCfg::default();
    let default_screen_cfg = PlanarScreenGuardCfg::default();
    let mut directives = Vec::new();
    let mut purge_sensor_residue = false;

    for (idx, roi) in rois.iter().enumerate() {
        let surface_id = idx as u32;
        let Some(patch3d) = fit_planar_patch(roi, depth, normals, intr, pose, rig, &fit_cfg) else {
            continue;
        };
        if let ReadabilityVerdict::NeedReposition(app) =
            evaluate_readability(&patch3d, intr, pose, &gate_cfg)
        {
            directives.push(FieldHudDirective::Reposition {
                target_quad_px: patch3d.corners_px,
                guide_ko: format_hud_guide_ko(&app),
                approach: app,
            });
            continue;
        }
        let Some(hom) = compute_planar_homography(&patch3d, intr, pose, 5.5, 4.0, 2016) else {
            continue;
        };
        let mut rect = unwarp_planar_patch(rgb, hom);

        let is_display = matches!(patch3d.hint, SurfaceClassHint::DisplayScreen)
            || matches!(patch3d.form_factor, PhysicalFormFactor::Display16x9);
        if is_display {
            let captured = privacy_epoch.load(Ordering::Acquire);
            let (models_opt, scfg) = match screen_guard {
                Some(sg) => (sg.models, &sg.cfg),
                None => (None, &default_screen_cfg),
            };
            match inspect_and_guard_3d_screen(
                rgb,
                &mut rect,
                &patch3d,
                models_opt,
                privacy_epoch,
                captured,
                scfg,
            )? {
                PlanarScreenOutcome::PrivacyRedacted {
                    zeroized_pixels,
                    reason,
                    trigger_sensor_purge: _,
                } => {
                    purge_sensor_residue = true;
                    directives.push(FieldHudDirective::PrivacyRedacted {
                        surface_id,
                        zeroized_pixels,
                        reason: format!("{reason:?}"),
                    });
                    continue;
                }
                PlanarScreenOutcome::Clear(_) => {}
            }
        }

        let Some(prep) = prepare_rectified_document(&rect, Baseline::Best, None) else {
            continue;
        };
        let mut callouts = Vec::new();
        let mut present_fields = Vec::new();
        let mut concat_repaired = String::new();
        let mut all_grounded = true;

        for cl in claims {
            let mut repaired = match scheme {
                Some(sc)
                    if cl.field.contains("code")
                        || cl.field.contains("id")
                        || cl.field.contains("container") =>
                {
                    repair_surface_identifier(&cl.raw_text, sc)
                }
                _ => cl.raw_text.clone(),
            };
            if !ledger_candidates.is_empty() && !cl.shadow_top3.is_empty() {
                match verify_rectified_ledger_shadow(
                    &repaired,
                    &cl.shadow_top3,
                    ledger_candidates,
                ) {
                    LatticeVerifyDecision::Verified { matched_token } => repaired = matched_token,
                    LatticeVerifyDecision::RejectedUngrounded { .. } => all_grounded = false,
                }
            }
            if scheme == Some(CodeScheme::Iso6346Container)
                && repaired.len() == 11
                && !verify_iso6346_check_digit(&repaired)
            {
                all_grounded = false;
            }
            let (checked_text, grounded, _) = if !source_evidence_text.is_empty() {
                draft_or_fallback(&repaired, source_evidence_text, &cl.raw_text)
            } else {
                (repaired.clone(), true, Vec::new())
            };
            all_grounded &= grounded;
            present_fields.push(cl.field.clone());
            if !concat_repaired.is_empty() {
                concat_repaired.push(' ');
            }
            concat_repaired.push_str(&checked_text);

            let (u0, v0, u1, v1) = cl.rect_bbox_px;
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
            let anchored = AnchoredField3D {
                category: cl.category.clone(),
                field: cl.field.clone(),
                text_repaired: checked_text,
                rect_bbox_px: cl.rect_bbox_px,
                image_quad_px,
                center_w,
                corners_w,
                view_obs: obs,
            };
            registry.upsert(&anchored, 8);
            callouts.push(anchored);
        }

        let pack_result = pack_input.map(|(pack, claimed_total, evidence_total)| {
            evaluate_pack(
                &PackCheckInput {
                    pack,
                    claimed_total,
                    evidence_total,
                    fields_present: present_fields,
                    ocr_predicted: concat_repaired.clone(),
                    ocr_reference: source_evidence_text.to_string(),
                },
                mode,
            )
        });

        directives.push(FieldHudDirective::VerifiedCallouts {
            surface_id,
            form_factor: format!("{:?}", patch3d.form_factor),
            virtual_table_cols: prep.virtual_cols,
            has_protected_titleblock: prep.has_protected_titleblock,
            pack_result,
            factcheck_grounded: all_grounded,
            callouts,
        });
    }

    if purge_sensor_residue {
        if let Some(ov) = overlay {
            ov.notify_external_privacy_hit();
        }
    }
    Ok(directives)
}

pub fn commit_verified_directives(
    pipeline: &StagedLouisPipeline,
    case_id: &str,
    directives: &[FieldHudDirective],
) -> Result<Option<IngestResult>> {
    let mut lines = Vec::new();
    for d in directives {
        if let FieldHudDirective::VerifiedCallouts { callouts, .. } = d {
            for c in callouts {
                lines.push(format!(
                    "{}:{}={}@[ {:.3},{:.3},{:.3} ]",
                    c.category,
                    c.field,
                    c.text_repaired,
                    c.center_w[0],
                    c.center_w[1],
                    c.center_w[2]
                ));
            }
        }
    }
    if lines.is_empty() {
        return Ok(None);
    }
    Ok(Some(pipeline.ingest_field_records(case_id, lines)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_frontal_plane(
        intr: &Intrinsics,
        pose: &Pose,
        z_dist_m: f64,
        half_u_m: f64,
        half_v_m: f64,
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
            eye[0] + z_dist_m * fwd[0] as f64,
            eye[1] + z_dist_m * fwd[1] as f64,
            eye[2] + z_dist_m * fwd[2] as f64,
        ];

        let mut min_x = w;
        let mut max_x = 0u32;
        let mut min_y = h;
        let mut max_y = 0u32;

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
                if u.abs() <= half_u_m && v.abs() <= half_v_m {
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

    #[test]
    fn field3d_kiosk_block_reposition_monitor_zeroize_and_fast_ledger_commit() {
        let intr = Intrinsics::pinhole(640, 480, 22.0);
        let mut pose = Pose::at_pos(0.0, 1.5, 0.0);
        pose.yaw = std::f32::consts::FRAC_PI_2;
        let epoch = Arc::new(AtomicU64::new(0));
        let mut reg = PlanarAnchorRegistry::new();

        // 1. CounterKiosk must immediately fail with KioskForbidden.
        let (dm_near, roi_near) = synth_frontal_plane(
            &intr,
            &pose,
            0.28,
            0.035,
            0.0225,
            SurfaceClassHint::IndustrialNameplate,
        );
        let mut rgb = Rgb::new(640, 480, vec![180u8; 640 * 480 * 3]);
        let err = inspect_field_frame(
            DeviceRole::CounterKiosk,
            &mut rgb,
            FieldDepthInput::AlignedMonocular {
                depth: &dm_near,
                normals: None,
            },
            &[roi_near.clone()],
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
        )
        .unwrap_err();
        assert!(err.to_string().contains("Kiosk"), "unexpected error: {err}");

        // 2. Distant plaque triggers Reposition (0 VLM calls) with Korean guidance.
        let (dm_far, roi_far) = synth_frontal_plane(
            &intr,
            &pose,
            2.6,
            0.09,
            0.06,
            SurfaceClassHint::IndustrialNameplate,
        );
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
        )
        .unwrap();
        assert!(
            matches!(dirs_far.first(), Some(FieldHudDirective::Reposition { guide_ko, .. }) if !guide_ko.is_empty()),
            "expected Reposition directive, got {dirs_far:?}"
        );

        // 3. 3D DisplayScreen with screen_guard = None triggers fail-closed quad zeroize & epoch bump.
        let (dm_scr, roi_scr) = synth_frontal_plane(
            &intr,
            &pose,
            0.55,
            0.080,
            0.045,
            SurfaceClassHint::DisplayScreen,
        );
        let mut ov = OverlayController::new(64, 64, 0.15);
        assert_eq!(ov.privacy_epoch(), 0);
        let dirs_scr = inspect_field_frame(
            DeviceRole::PersonalPhone,
            &mut rgb,
            FieldDepthInput::AlignedMonocular {
                depth: &dm_scr,
                normals: None,
            },
            &[roi_scr],
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
            Some(&mut ov),
            &mut reg,
        )
        .unwrap();
        assert!(
            matches!(
                dirs_scr.first(),
                Some(FieldHudDirective::PrivacyRedacted { zeroized_pixels, .. }) if *zeroized_pixels > 1000
            ),
            "expected PrivacyRedacted, got {dirs_scr:?}"
        );
        assert_eq!(rgb.px(320, 240), [0, 0, 0]);
        assert_eq!(rgb.px(0, 0), [180, 180, 180]);
        assert_eq!(ov.privacy_epoch(), 1);

        // 4. Verified callouts + ledger commit keeps mutex hold < 2,000 us.
        let mut rgb_ok = Rgb::new(640, 480, vec![210u8; 640 * 480 * 3]);
        let claim = FieldClaimCandidate {
            category: "kks".into(),
            field: "kks_code".into(),
            raw_text: "10LAB1OAPOO1".into(),
            rect_bbox_px: (10, 10, 80, 40),
            shadow_top3: Vec::new(),
        };
        let dirs_ok = inspect_field_frame(
            DeviceRole::PersonalPhone,
            &mut rgb_ok,
            FieldDepthInput::AlignedMonocular {
                depth: &dm_near,
                normals: None,
            },
            &[roi_near],
            &intr,
            &pose,
            &[claim],
            &[],
            "10LAB10AP001",
            None,
            Some(CodeScheme::Kks),
            EngineMode::RulesOnly,
            None,
            &epoch,
            None,
            &mut reg,
        )
        .unwrap();
        assert!(
            matches!(dirs_ok.first(), Some(FieldHudDirective::VerifiedCallouts { .. })),
            "expected VerifiedCallouts, got {dirs_ok:?}"
        );
        let pipe = StagedLouisPipeline::new(4096);
        let ingest = commit_verified_directives(&pipe, "CASE-FIELD3D-01", &dirs_ok)
            .unwrap()
            .expect("ingested callouts");
        assert!(ingest.max_lock_hold_us < 2_000);
        assert_eq!(ingest.block_count, 1);
    }
}
