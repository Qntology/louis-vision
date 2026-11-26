//! Layer 2 (`Phase 4`): `vision-sandbox` Cargo Track (`P-C1..P-C4`),
//! `XR Stage Live` 36-cell matrix (`X1..X8`), `Gate UI-HARING`, and `:8788` 5-tool MCP tests.

use lv_doc::vision::symbols::{
    candidates, crop_square, normalize_vec, CandidateParams, SymbolDict, SymbolFeature,
};
use lv_domain::CodeScheme;
use lv_planar::anchor3d::PlanarAnchorRegistry;
use lv_planar::gate::ReadabilityBlockReason;
use lv_planar::homography::{compute_planar_homography, unwarp_planar_patch};
use lv_spatial::camera::{Intrinsics, Pose};
use lv_spatial::fit3::{Similarity, V3};
use lv_spatial::frame::{DepthKind, DepthMap, Rgb};
use lv_spatial::planar::{fit_planar_patch, PlanarCandidateRoi, PlanarFitCfg, SurfaceClassHint};
use lv_spatial::stereo::StereoRig;
use lv_spatial::zkey::{SandboxBackground, ZKeyCfg};
use sandbox_core::rig::haring_dancer::{build_haring_dancer_rig, render_haring_orthographic_rgb};
use sandbox_core::rig::venue_cad::{build_stage_venue_urdf, StageVenueSpec};
use sandbox_runtime::cargo::CargoAction;
use sandbox_runtime::haring_visual_audit::verify_haring_screenshot_invariants;
use sandbox_runtime::mcp::{dispatch_stage_synth_tool, handle as mcp_handle, tools as mcp_tools};
use sandbox_runtime::planar_read::{
    cargo_action_for_label, step_parcel_planar_inspection, DecodedSurfaceSpan, RobotPlanarOutcome,
};
use sandbox_runtime::sds::Provenance;
use serde_json::{json, Value};
use vision_sandbox::calibration::SandboxCalibration;
use vision_sandbox::interlock::{
    verify_audit_chain, CueTriggerSource, InterlockDecision, InterlockInputs, SandboxHazardObb,
    SandboxInterlock, INTERLOCK_HALT_OSC,
};
use vision_sandbox::rig_capture::{MAX_SANDBOX_TALLY_LUMA_PCT, STEREO_KEYFRAME_QUALITY};
use vision_sandbox::sandbox_stereo::StereoSandboxLive;
use vision_sandbox::show_io::gpu_share::{build_zero_copy_descriptor, GpuShareBackend};
use vision_sandbox::show_io::ndi_stream::pack_ndi_video_alpha;
use vision_sandbox::show_io::openxr_bridge::basis_to_unit_quat;
use vision_sandbox::show_io::osc_udp::{decode_osc_message, encode_osc_message, OscArg};
use vision_sandbox::validate_local_socket;

fn identity_sim() -> Similarity {
    Similarity {
        s: 1.0,
        r: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        t: [0.0, 0.0, 0.0],
    }
}

fn candidate_descriptor_local(crop: &image::RgbImage, dim: usize) -> Vec<f32> {
    let mut vec = vec![0.0f32; dim];
    let (w, h) = crop.dimensions();
    if w == 0 || h == 0 || dim == 0 {
        return vec;
    }
    for y in 0..h {
        for x in 0..w {
            let p = crop.get_pixel(x, y).0;
            let lum =
                (p[0] as f32 * 0.299 + p[1] as f32 * 0.587 + p[2] as f32 * 0.114) / 255.0;
            let ink = (1.0 - lum).max(0.0);
            let idx = (((y * dim as u32 / h) * 7 + (x * dim as u32 / w) * 13) as usize) % dim;
            vec[idx] += ink;
        }
    }
    normalize_vec(&mut vec);
    vec
}

fn make_parcel_scene(
    z_m: f32,
    draw_stamp: bool,
) -> (Intrinsics, Pose, Rgb, DepthMap, PlanarCandidateRoi) {
    let (w, h) = (640u32, 480u32);
    // f_px ~= 2277 px so z = 0.45m gives ~5.06 px/mm (>= 4.0) and z = 3.10m gives ~0.73 px/mm (< 4.0)
    let intr = Intrinsics::pinhole(w, h, 16.0);
    let pose = Pose {
        x: 0.0,
        z: 0.0,
        yaw: 0.0,
        eye_m: 0.0,
    };
    let mut rgb_buf = vec![245u8; (w * h * 3) as usize];
    if draw_stamp {
        // Draw a 56x56 patterned black-ink symbol inside the ROI (fill ~0.31, in [0.04..0.62])
        for y in 180u32..236 {
            for x in 200u32..256 {
                let border = x < 204 || x >= 252 || y < 184 || y >= 232;
                let diag = ((x - 200) == (y - 180)) || ((55 - (x - 200)) == (y - 180));
                if border || diag {
                    let o = ((y * w + x) * 3) as usize;
                    rgb_buf[o..o + 3].copy_from_slice(&[12, 12, 12]);
                }
            }
        }
        // Dark titleblock strokes in bottom-right of ROI
        for y in 330u32..360 {
            for x in 420u32..490 {
                if y % 6 < 2 {
                    let o = ((y * w + x) * 3) as usize;
                    rgb_buf[o..o + 3].copy_from_slice(&[20, 20, 20]);
                }
            }
        }
    }
    let rgb = Rgb::new(w, h, rgb_buf);
    let depth = DepthMap {
        width: w,
        height: h,
        kind: DepthKind::Metric,
        data: vec![z_m; (w * h) as usize],
    };
    let roi = PlanarCandidateRoi {
        x0: 120,
        y0: 90,
        x1: 520,
        y1: 390,
        hint: SurfaceClassHint::GenericPlanar,
    };
    (intr, pose, rgb, depth, roi)
}

fn build_dict_from_scene_stamp(
    class_name: &str,
    rgb: &Rgb,
    depth: &DepthMap,
    roi: &PlanarCandidateRoi,
    intr: &Intrinsics,
    pose: &Pose,
) -> SymbolDict {
    let patch = fit_planar_patch(roi, depth, None, intr, pose, None, &PlanarFitCfg::default())
        .expect("fit_planar_patch should succeed on stamped scene");
    let hom = compute_planar_homography(&patch, intr, pose, 5.5, 4.0, 2016)
        .expect("homography should succeed");
    let rect = unwarp_planar_patch(rgb, hom);
    let rect_rgb = rect.to_rgb_image().expect("rect RGB image");
    let gray = image::DynamicImage::ImageRgb8(rect_rgb).to_luma8();
    let gray_rgb = image::RgbImage::from_fn(gray.width(), gray.height(), |x, y| {
        let v = gray.get_pixel(x, y)[0];
        image::Rgb([v, v, v])
    });
    let cands = candidates(&gray_rgb, &CandidateParams::default());
    assert!(
        !cands.is_empty(),
        "stamped symbol must produce at least one A0 candidate in {}x{} rectified image",
        gray_rgb.width(),
        gray_rgb.height()
    );
    let sq = crop_square(&gray_rgb, cands[0].bbox);
    let sym_vec = candidate_descriptor_local(&sq, 64);
    let mut neg_vec: Vec<f32> = sym_vec.iter().map(|&v| -v).collect();
    normalize_vec(&mut neg_vec);
    SymbolDict::from_legend_samples(
        SymbolFeature::Pooled,
        vec![class_name.to_string(), "unrelated_class".to_string()],
        &[(0, sym_vec), (1, neg_vec)],
    )
    .expect("SymbolDict build from scene crop")
}

#[test]
fn p_c1_to_p_c4_planar_read_cargo_track() {
    let mut reg = PlanarAnchorRegistry::new();

    // P-C1 (far): z = 3.10 m (< 4.0 px/mm) -> Reposition with 0 decoder calls
    let (intr_far, pose_far, rgb_far, depth_far, roi_far) = make_parcel_scene(3.10, false);
    let mut decoder_calls = 0usize;
    let out_far = step_parcel_planar_inspection(
        Provenance::Experience,
        Provenance::Spec,
        &rgb_far,
        &depth_far,
        None,
        &roi_far,
        &intr_far,
        &pose_far,
        None,
        None,
        &["MSCU1134660"],
        Some(CodeScheme::Iso6346Container),
        &mut reg,
        |_| {
            decoder_calls += 1;
            Vec::new()
        },
    );
    assert_eq!(decoder_calls, 0, "P-C1 far target must make 0 VLM/decoder calls");
    match out_far {
        RobotPlanarOutcome::Reposition { approach, .. } => {
            assert_eq!(approach.reason, ReadabilityBlockReason::LowEffectiveDpi);
            assert!(approach.observed_px_per_mm < 4.0);
        }
        other => panic!("expected Reposition on far parcel, got {other:?}"),
    }

    // P-C1 (close): z = 0.35 m (~6.5 px/mm >= 5.5) -> Inspected, valid ISO 6346 MSCU1134660 -> Take
    let (intr_ok, pose_ok, rgb_ok, depth_ok, roi_ok) = make_parcel_scene(0.35, true);
    let out_ok = step_parcel_planar_inspection(
        Provenance::Experience,
        Provenance::Spec,
        &rgb_ok,
        &depth_ok,
        None,
        &roi_ok,
        &intr_ok,
        &pose_ok,
        None,
        None,
        &["MSCU1134660"],
        Some(CodeScheme::Iso6346Container),
        &mut reg,
        |_| {
            vec![DecodedSurfaceSpan {
                field: "container".into(),
                raw_text: "MSCUll3466O".into(), // OCR confusion -> repaired to MSCU1134660
                rect_bbox_px: (20, 20, 180, 56),
                shadow_top3: Vec::new(),
            }]
        },
    );
    match out_ok {
        RobotPlanarOutcome::Inspected {
            verdict, callouts, ..
        } => {
            assert!(verdict.iso6346_valid);
            assert!(!verdict.ledger_mismatch);
            assert_eq!(cargo_action_for_label(&verdict), CargoAction::Take);
            assert!(!callouts.is_empty());
        }
        other => panic!("expected Inspected on close parcel, got {other:?}"),
    }

    // P-C2: real SymbolDict detects "fragile_parcel" -> grip 0.65, accel 0.55, Take
    let fragile_dict = build_dict_from_scene_stamp(
        "fragile_parcel",
        &rgb_ok,
        &depth_ok,
        &roi_ok,
        &intr_ok,
        &pose_ok,
    );
    let out_fragile = step_parcel_planar_inspection(
        Provenance::Experience,
        Provenance::Spec,
        &rgb_ok,
        &depth_ok,
        None,
        &roi_ok,
        &intr_ok,
        &pose_ok,
        None,
        Some(&fragile_dict),
        &["MSCU1134660"],
        Some(CodeScheme::Iso6346Container),
        &mut reg,
        |_| {
            vec![DecodedSurfaceSpan {
                field: "container".into(),
                raw_text: "MSCU1134660".into(),
                rect_bbox_px: (20, 20, 180, 56),
                shadow_top3: Vec::new(),
            }]
        },
    );
    match out_fragile {
        RobotPlanarOutcome::Inspected { verdict, .. } => {
            assert!(verdict.fragile_detected);
            assert!((verdict.grip_force_scale - 0.65).abs() < 1e-5);
            assert!((verdict.accel_cap_scale - 0.55).abs() < 1e-5);
            assert_eq!(cargo_action_for_label(&verdict), CargoAction::Take);
        }
        other => panic!("expected Inspected fragile outcome, got {other:?}"),
    }

    // P-C3: real SymbolDict detects "hazard_flammable" -> CargoAction::Refuse; plus ledger mismatch -> Refuse
    let hazard_dict = build_dict_from_scene_stamp(
        "hazard_flammable",
        &rgb_ok,
        &depth_ok,
        &roi_ok,
        &intr_ok,
        &pose_ok,
    );
    let out_haz = step_parcel_planar_inspection(
        Provenance::Experience,
        Provenance::Spec,
        &rgb_ok,
        &depth_ok,
        None,
        &roi_ok,
        &intr_ok,
        &pose_ok,
        None,
        Some(&hazard_dict),
        &["MSCU1134660"],
        Some(CodeScheme::Iso6346Container),
        &mut reg,
        |_| {
            vec![DecodedSurfaceSpan {
                field: "container".into(),
                raw_text: "MSCU1134660".into(),
                rect_bbox_px: (20, 20, 180, 56),
                shadow_top3: Vec::new(),
            }]
        },
    );
    match out_haz {
        RobotPlanarOutcome::Inspected { verdict, .. } => {
            assert!(verdict.hazard_detected);
            assert_eq!(cargo_action_for_label(&verdict), CargoAction::Refuse);
        }
        other => panic!("expected Inspected hazard outcome, got {other:?}"),
    }

    let out_mismatch = step_parcel_planar_inspection(
        Provenance::Experience,
        Provenance::Spec,
        &rgb_ok,
        &depth_ok,
        None,
        &roi_ok,
        &intr_ok,
        &pose_ok,
        None,
        None,
        &["MSCU1134660"],
        Some(CodeScheme::Iso6346Container),
        &mut reg,
        |_| {
            vec![DecodedSurfaceSpan {
                field: "container".into(),
                raw_text: "MSCU1134661".into(), // Invalid check digit / wrong ID
                rect_bbox_px: (20, 20, 180, 56),
                shadow_top3: Vec::new(),
            }]
        },
    );
    match out_mismatch {
        RobotPlanarOutcome::Inspected { verdict, .. } => {
            assert!(verdict.ledger_mismatch);
            assert_eq!(cargo_action_for_label(&verdict), CargoAction::Refuse);
        }
        other => panic!("expected Inspected ledger mismatch outcome, got {other:?}"),
    }

    // P-C4: Provenance::{Truth, Reference} MUST abstain immediately with 0 decoder calls; wipe zeroes buffer
    for bad_prov in [Provenance::Truth, Provenance::Reference] {
        let mut called = 0usize;
        let ab = step_parcel_planar_inspection(
            bad_prov,
            Provenance::Spec,
            &rgb_ok,
            &depth_ok,
            None,
            &roi_ok,
            &intr_ok,
            &pose_ok,
            None,
            None,
            &["MSCU1134660"],
            Some(CodeScheme::Iso6346Container),
            &mut reg,
            |_| {
                called += 1;
                Vec::new()
            },
        );
        assert_eq!(called, 0);
        assert!(matches!(ab, RobotPlanarOutcome::AbstainedProvenance));
    }

    let patch = fit_planar_patch(&roi_ok, &depth_ok, None, &intr_ok, &pose_ok, None, &PlanarFitCfg::default()).unwrap();
    let hom = compute_planar_homography(&patch, &intr_ok, &pose_ok, 5.5, 4.0, 2016).unwrap();
    let mut rect = unwarp_planar_patch(&rgb_ok, hom);
    rect.wipe();
    assert!(rect.rgb.iter().all(|&b| b == 0));
}

#[test]
fn x1_to_x8_xr_stage_live_36cell_matrix() {
    let perturbations = [
        "nominal",
        "truss_vibration_3mm",
        "glare_nan_holes_6pct",
        "crossing_performers",
        "low_luma_shift",
        "subpx_disparity_jitter",
    ];
    let rig = StereoRig {
        focal_px: 300.0,
        baseline_m: 0.12,
        subpixel_px: 0.15,
    };
    let venue = build_stage_venue_urdf(&StageVenueSpec::default()).unwrap();
    assert!(venue.roundtrip_max_err_m < 1e-4);

    let mut passed_cells = 0usize;
    let rig_markers: [V3; 3] = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 0.0, 2.0]];
    let stage_markers: [V3; 3] = [[1.0, 0.2, 0.5], [3.0, 0.2, 0.5], [1.0, 0.2, 2.5]];

    for (p_idx, pert) in perturbations.iter().enumerate() {
        let (dw, dh) = (60u32, 40u32);
        let bg_frames: Vec<DepthMap> = (0..30)
            .map(|f_idx| {
                let mut data = vec![6.0f32; (dw * dh) as usize];
                for px in 0..data.len() {
                    if *pert == "truss_vibration_3mm" {
                        let sign = if (f_idx + px) % 2 == 0 { 1.0 } else { -1.0 };
                        data[px] += 0.003 * sign;
                    } else if *pert == "subpx_disparity_jitter" {
                        data[px] += 0.002 * (((f_idx % 5) as f32) - 2.0);
                    }
                }
                DepthMap {
                    width: dw,
                    height: dh,
                    kind: DepthKind::Metric,
                    data,
                }
            })
            .collect();
        let bg_refs: Vec<&DepthMap> = bg_frames.iter().collect();
        let bg_luma = if *pert == "low_luma_shift" { 18u8 } else { 30u8 };
        let bg_rgb = Rgb::new(120, 80, vec![bg_luma; 120 * 80 * 3]);

        // Cell 1 (X1): 29 frames rejected -> 30 frames calibrated with SE(3) s = 1.0 & rmse < 0.020 m
        assert!(SandboxCalibration::calibrate_rigid(
            &rig_markers,
            &stage_markers,
            &bg_refs[..29],
            Some(bg_rgb.clone()),
            0.03,
        )
        .is_err());
        let cal = SandboxCalibration::calibrate_rigid(
            &rig_markers,
            &stage_markers,
            &bg_refs[..30],
            Some(bg_rgb),
            0.03,
        )
        .unwrap();
        assert!((cal.rig_to_sandbox.s - 1.0).abs() <= 1e-9);
        assert!(cal.rmse_m < 0.020);
        passed_cells += 1;

        // Cell 2 (X2/X5): Z-Keying performer tracking + crossing ID preservation + zero swap state
        let intr = Intrinsics::pinhole(120, 80, 70.0);
        let pose = Pose {
            x: 0.0,
            z: 0.0,
            yaw: 0.0,
            eye_m: 1.5,
        };
        let bg = SandboxBackground::capture_median(
            &bg_refs[..30],
            Some(Rgb::new(120, 80, vec![bg_luma; 120 * 80 * 3])),
            0.03,
        )
        .unwrap();
        let mut live = StereoSandboxLive::new(
            rig,
            Default::default(),
            None,
            bg,
            ZKeyCfg::default(),
            intr,
            pose,
            identity_sim(),
        );
        let mut fg_depth = DepthMap {
            width: dw,
            height: dh,
            kind: DepthKind::Metric,
            data: vec![6.0f32; (dw * dh) as usize],
        };
        let mut fg_rgb = vec![bg_luma; 120 * 80 * 3];
        for y in 8u32..32 {
            for x in 14u32..24 {
                let hole = *pert == "glare_nan_holes_6pct" && (x + y) % 16 == 0;
                fg_depth.data[(y * dw + x) as usize] = if hole { f32::NAN } else { 2.1 };
                for ry in (y * 2)..(y * 2 + 2) {
                    for rx in (x * 2)..(x * 2 + 2) {
                        let i = ((ry * 120 + rx) * 3) as usize;
                        fg_rgb[i..i + 3].copy_from_slice(&[225, 225, 225]);
                    }
                }
            }
            for x in 30u32..40 {
                fg_depth.data[(y * dw + x) as usize] = 4.1;
                for ry in (y * 2)..(y * 2 + 2) {
                    for rx in (x * 2)..(x * 2 + 2) {
                        let i = ((ry * 120 + rx) * 3) as usize;
                        fg_rgb[i..i + 3].copy_from_slice(&[225, 225, 225]);
                    }
                }
            }
        }
        let f_out = live
            .process_calibrated_depth(&Rgb::new(120, 80, fg_rgb), &fg_depth)
            .unwrap();
        assert!(f_out.performers.len() >= 2, "pert={pert}");
        assert_ne!(f_out.performers[0].id, f_out.performers[1].id);
        passed_cells += 1;

        // Cell 3 (X3): 3-Step Interlock + OBB-Lift-C -> /louis/interlock/halt + hash chain
        let dir = std::env::temp_dir().join(format!("xr_nn_il_{}_{}", std::process::id(), p_idx));
        let _ = std::fs::remove_dir_all(&dir);
        let mut il = SandboxInterlock::new_persistent(
            vec![SandboxHazardObb {
                name: venue.obb_lift_c.name.clone(),
                min_xyz: venue.obb_lift_c.min_xyz,
                max_xyz: venue.obb_lift_c.max_xyz,
            }],
            &dir,
            "xr_nn_il",
            128,
        )
        .unwrap();
        let steady = InterlockInputs {
            active_view_valid_ratios: vec![0.91, 0.89],
            metric_points_m: vec![[3.5, 1.0, 2.5]],
            fusion_residual_m: Some(if *pert == "truss_vibration_3mm" { 0.008 } else { 0.005 }),
            residual_history_m: vec![0.006; 16],
            residual_sds_sd_m: Some(0.002),
        };
        let d_ok = il
            .dispatch_cue(CueTriggerSource::AutoTimecode, "go_01", &steady)
            .unwrap();
        assert!(d_ok.allowed && d_ok.osc_address == "/louis/cue/go_01");

        let mut intrude = steady.clone();
        intrude.metric_points_m.push([0.0, 0.0, 1.0]); // Inside OBB-Lift-C
        let h_auto = il
            .dispatch_cue(CueTriggerSource::AutoTimecode, "go_02", &intrude)
            .unwrap();
        let h_rem = il
            .dispatch_cue(CueTriggerSource::SandboxCueRemote, "go_02", &intrude)
            .unwrap();
        assert!(!h_auto.allowed && h_auto.osc_address == INTERLOCK_HALT_OSC);
        assert!(!h_rem.allowed && h_rem.osc_address == INTERLOCK_HALT_OSC);
        assert!(matches!(h_auto.decision, InterlockDecision::Halt { .. }));

        let log_path = il.audit_path().unwrap();
        assert!(verify_audit_chain(&log_path).unwrap() >= 3);
        drop(il);
        let _ = std::fs::remove_dir_all(&dir);
        passed_cells += 1;

        // Cell 4 (X4/X8): Spout2 + NDI + OSC + OpenXR quat + tally <= 8% + WAN block + blackout <= 16_666 us
        let gpu_desc = build_zero_copy_descriptor(
            GpuShareBackend::Spout2,
            "VisionSandbox_ID",
            &f_out.matte,
            &f_out.id_matte_rgba,
            &f_out.depth,
            f_out.frame_index,
        )
        .unwrap();
        assert_eq!(gpu_desc.color_format, "RGBA8_UNORM");
        assert!(pack_ndi_video_alpha(
            "Stage",
            "8.8.8.8:5961".parse().unwrap(),
            &Rgb::new(120, 80, vec![0; 120 * 80 * 3]),
            &f_out.matte,
        )
        .is_err());
        assert!(validate_local_socket("8.8.8.8:9000".parse().unwrap(), "osc").is_err());
        assert!(validate_local_socket("127.0.0.1:9000".parse().unwrap(), "osc").is_ok());
        assert!(validate_local_socket("192.168.10.50:5568".parse().unwrap(), "sacn").is_ok());
        let pkt = encode_osc_message(INTERLOCK_HALT_OSC, &[OscArg::Int(p_idx as i32)]).unwrap();
        let (addr, args) = decode_osc_message(&pkt).unwrap();
        assert_eq!(addr, INTERLOCK_HALT_OSC);
        assert_eq!(args, vec![OscArg::Int(p_idx as i32)]);
        let q = basis_to_unit_quat([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
        let qnorm = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        assert!((qnorm - 1.0).abs() < 1e-4);
        assert!(MAX_SANDBOX_TALLY_LUMA_PCT <= 8 && STEREO_KEYFRAME_QUALITY >= 90.0);
        let blk = dispatch_stage_synth_tool(
            "cue_timeline_synth",
            &json!({"bg_frames": 30, "blackout": true}),
        )
        .unwrap()
        .unwrap();
        assert!(blk["blackout_active"].as_bool().unwrap());
        assert!(blk["blackout_latency_us"].as_u64().unwrap() <= 16_666);
        assert_eq!(blk["weight_swaps_1000f"].as_u64().unwrap(), 0);
        passed_cells += 1;

        // Cell 5 (X5): sACN Univ 64 non-dimmable + Laser BAM 0% + WAN IP rejection
        let rig_ok = dispatch_stage_synth_tool(
            "stage_rig_edit",
            &json!({
                "pyro_universe": 64,
                "non_dimmable": true,
                "controller_ip": "192.168.50.20",
                "laser_in_frustum": true
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(rig_ok["laser_bam_pct"], 0.0);
        assert_eq!(rig_ok["backstage_monitor_45deg_nonzero_px"], 0);
        assert!(dispatch_stage_synth_tool(
            "stage_rig_edit",
            &json!({"pyro_universe": 63, "non_dimmable": true, "controller_ip": "192.168.50.20"}),
        )
        .unwrap()
        .is_err());
        assert!(dispatch_stage_synth_tool(
            "stage_rig_edit",
            &json!({"pyro_universe": 64, "non_dimmable": true, "controller_ip": "8.8.8.8"}),
        )
        .unwrap()
        .is_err());
        passed_cells += 1;

        // Cell 6 (X6): 29f reject vs mc_speech_vamp / hoist_chain_grind / ballot 0 false triggers
        assert!(dispatch_stage_synth_tool("cue_timeline_synth", &json!({"bg_frames": 29}))
            .unwrap()
            .is_err());
        let vamp = dispatch_stage_synth_tool(
            "cue_timeline_synth",
            &json!({"bg_frames": 30, "audio_event": "mc_speech_vamp"}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(vamp["vamp_hold_active"], true);
        assert_eq!(vamp["false_vamp_on_ballad"], 0);
        let grind = dispatch_stage_synth_tool(
            "cue_timeline_synth",
            &json!({"bg_frames": 30, "audio_event": "hoist_chain_grind"}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(grind["interlock_halt"], true);
        assert_eq!(grind["osc_address"], "/louis/interlock/halt");
        passed_cells += 1;
    }

    assert_eq!(passed_cells, 36);
}

#[test]
fn gate_ui_haring_4view_and_mcp_8788_5tools() {
    let tool_list = mcp_tools();
    let names: Vec<&str> = tool_list
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    for expected in [
        "planar_stereo_render",
        "venue_build",
        "stage_rig_edit",
        "haring_crew_spawn",
        "cue_timeline_synth",
    ] {
        assert!(names.contains(&expected), "missing :8788 tool {expected}");
    }

    let dispatcher = |name: &str, args: &Value| -> Result<Value, String> {
        dispatch_stage_synth_tool(name, args)
            .unwrap_or_else(|| Err(format!("unhandled tool {name}")))
    };

    // 1. planar_stereo_render (ill-readable < 5.5 -> ApproachForRead, vlm_calls == 0; readable -> ok)
    let r_far = mcp_handle(
        &dispatcher,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "planar_stereo_render", "arguments": {"px_per_mm": 3.1, "surface_kind": "plaque"}}}),
    )
    .unwrap();
    let far_val: Value = serde_json::from_str(r_far["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(far_val["outcome"], "ApproachForRead");
    assert_eq!(far_val["vlm_calls"], 0);

    // 2. venue_build
    let r_ven = mcp_handle(
        &dispatcher,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "venue_build", "arguments": {}}}),
    )
    .unwrap();
    let ven_val: Value = serde_json::from_str(r_ven["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(ven_val["ok"], true);
    assert!(ven_val["build"]["roundtrip_max_err_m"].as_f64().unwrap() < 1e-4);

    // 3. stage_rig_edit
    let r_rig = mcp_handle(
        &dispatcher,
        &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "stage_rig_edit", "arguments": {"pyro_universe": 64, "non_dimmable": true, "controller_ip": "10.0.10.5", "laser_in_frustum": true}}}),
    )
    .unwrap();
    let rig_val: Value = serde_json::from_str(r_rig["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(rig_val["laser_bam_pct"], 0.0);

    // 4. haring_crew_spawn (5 wallets x 4 angles verified by Gate UI-HARING)
    let r_crew = mcp_handle(
        &dispatcher,
        &json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "haring_crew_spawn", "arguments": {}}}),
    )
    .unwrap();
    let crew_val: Value = serde_json::from_str(r_crew["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(crew_val["crew_count"], 5);
    assert_eq!(crew_val["bg_frames_captured"], 30);

    // 5. cue_timeline_synth (29f error + 30f obb_intrusion halt)
    let r_cue_err = mcp_handle(
        &dispatcher,
        &json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "cue_timeline_synth", "arguments": {"bg_frames": 29}}}),
    )
    .unwrap();
    assert_eq!(r_cue_err["result"]["isError"], true);

    let r_cue_ok = mcp_handle(
        &dispatcher,
        &json!({"jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": {"name": "cue_timeline_synth", "arguments": {"bg_frames": 30, "obb_intrusion": true, "blackout": true}}}),
    )
    .unwrap();
    let cue_val: Value = serde_json::from_str(r_cue_ok["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(cue_val["interlock_halt"], true);
    assert_eq!(cue_val["osc_address"], "/louis/interlock/halt");

    // 6. Direct Gate UI-HARING check from deterministic wallet derivation
    let bh = lv_sds::blockies_haring::derive_blockies_haring_spec("0xAbCDef012345678901234567890123456789AbCd");
    assert_eq!(bh.outline_hex, "#000000");
    let body = [255u8, 196, 0];
    let bg = [242u8, 244, 247];
    let rig = build_haring_dancer_rig("HARING-DIRECT", &bh.canonical_seed_0x, body, 1.0);
    for angle in [0u16, 45, 90, 180] {
        let img = render_haring_orthographic_rgb(&rig, 256, 256, angle, bg);
        let rep = verify_haring_screenshot_invariants(&img, 256, 256, angle, body, bg, true).unwrap();
        assert!(rep.body_color_purity_ratio >= 0.995);
        assert_eq!(rep.outline_gap_pixels, 0);
        if angle != 90 {
            assert!(rep.floating_tick_components >= 4);
        }
    }
}
