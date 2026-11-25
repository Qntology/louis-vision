use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use candle_core::{DType, Device, Shape, Tensor};
use candle_nn::var_builder::SimpleBackend;
use candle_nn::VarBuilder;
use lv_core::siglip2::antialias_bilinear_weights;
use lv_spatial::align::{fit_ground, ground_align, ground_depth, AlignCfg, AlignSpace};
use lv_spatial::camera::{Intrinsics, Pose};
use lv_spatial::decide::{DecideCfg, Decider, Decision, Sighting};
use lv_spatial::depth::{DepthModel, SceneHeads};
use lv_spatial::egoexo::{fuse, ViewObs};
use lv_spatial::fit3::{align_rigid, pa_mpjpe};
use lv_spatial::frame::{DepthKind, DepthMap, Mask, Rgb};
use lv_spatial::item::item_pose;
use lv_spatial::locate::{label_heats, LabelTexts, LocateCfg, LocateScoring};
use lv_spatial::pos2::{Pos2Filter, Pos2Model, Pos2Obs};
use lv_spatial::pyramid::{cell_geometry, gate_points, snap_points, CellGateCfg, Pyramid, PyramidCfg, SnapCfg, View};
use lv_spatial::stairs::{read_step, StepCfg};
use lv_spatial::stereo::{sgbm, SgbmCfg, SgbmDepth, StereoRig};
use lv_spatial::tips::{maybe_load_monocular, InputGrid, TipsConfig, TipsDepth, ADE_CLASSES};
use lv_spatial::track::{StereoSpatialPipeline, StereoSpatialTier};
use lv_spatial::zkey::{StageBackground, StageZKeyTracker, ZKeyCfg};

fn smooth_texture(w: u32, h: u32, seed: u64) -> Vec<f32> {
    let mut raw = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let mut z = ((y as u64) << 32) ^ (x as u64) ^ seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            raw.push(40.0 + ((z & 0xFFFF) as f32 / 65535.0) * 165.0);
        }
    }
    let mut nxt = raw.clone();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = (y * w + x) as usize;
            nxt[i] = 0.5 * raw[i]
                + 0.125
                    * (raw[i - 1]
                        + raw[i + 1]
                        + raw[i - w as usize]
                        + raw[i + w as usize]);
        }
    }
    nxt
}

fn sample_tex(tex: &[f32], w: u32, x: f32, y: u32) -> f32 {
    let xc = x.clamp(0.0, (w - 1) as f32);
    let x0 = xc.floor() as u32;
    let x1 = (x0 + 1).min(w - 1);
    let t = xc - x0 as f32;
    tex[(y * w + x0) as usize] * (1.0 - t) + tex[(y * w + x1) as usize] * t
}

fn make_textured_stereo_pair(w: u32, h: u32, shift_px: i32, bg_shift_px: i32, fg_rect: [u32; 4]) -> (Rgb, Rgb) {
    let fg = smooth_texture(w, h, 7);
    let bg = smooth_texture(w, h, 1);
    let in_fg = |x: f32, y: u32| -> bool {
        x >= fg_rect[0] as f32
            && x < fg_rect[2] as f32
            && y >= fg_rect[1]
            && y < fg_rect[3]
    };
    let mut left = Vec::with_capacity((w * h * 3) as usize);
    let mut right = Vec::with_capacity((w * h * 3) as usize);

    for y in 0..h {
        for x in 0..w {
            let vl = if in_fg(x as f32, y) {
                fg[(y * w + x) as usize]
            } else {
                bg[(y * w + x) as usize]
            };
            let ul = vl.round().clamp(0.0, 240.0) as u8;
            left.extend_from_slice(&[ul, ul, ul]);

            let xf = x as f32 + shift_px as f32;
            let vr = if in_fg(xf, y) {
                sample_tex(&fg, w, xf, y)
            } else {
                sample_tex(&bg, w, x as f32 + bg_shift_px as f32, y)
            };
            let ur = vr.round().clamp(0.0, 240.0) as u8;
            right.extend_from_slice(&[ur, ur, ur]);
        }
    }
    (Rgb::new(w, h, left), Rgb::new(w, h, right))
}

#[test]
fn gate_t1_spatial_a_sgbm_accuracy_honest_nan_inf_and_monocular_guard() {
    let rig = StereoRig {
        focal_px: 300.0,
        baseline_m: 0.12,
        subpixel_px: 0.15,
    };
    let mut cfg = SgbmCfg::default();
    cfg.per_l2 = 8; // 288x192 matching grid (L1=18x12 -> L2=36x24 -> L3=288x192) for 384x256 input
    cfg.speckle_size = 8;

    // 384x256 frame -> stereo grid 288x192 => f_grid = 225px, f*b = 27.0 m*px.
    // Foreground shift on grid = 22.5 px (full-res shift = 30 px) => Z_fg = 27.0 / 22.5 = 1.20 m!
    // Background shift on grid = 7.5 px (full-res shift = 10 px) => Z_bg = 27.0 / 7.5 = 3.60 m!
    let (left, right) = make_textured_stereo_pair(384, 256, 30, 10, [110, 70, 270, 210]);
    let out = sgbm(&left, &right, &rig, &cfg, None).expect("sgbm");
    assert_eq!(out.stats.width, 288);
    assert_eq!(out.stats.height, 192);
    let mut roi_vals = Vec::new();
    let mut near_ok = 0usize;
    let mut roi_total = 0usize;
    for y in 65..145 {
        for x in 95..190 {
            roi_total += 1;
            let d = out.depth.at(x, y);
            if d.is_finite() {
                roi_vals.push(d);
                if (d - 1.20).abs() <= 0.05 {
                    near_ok += 1;
                }
            }
        }
    }
    roi_vals.sort_by(f32::total_cmp);
    assert!(
        near_ok * 100 >= 85 * roi_total,
        "foreground ROI coverage too low: {near_ok}/{roi_total} near 1.20m; stats={:?}",
        out.stats
    );
    let med = roi_vals[roi_vals.len() / 2];
    assert!(
        (med - 1.20).abs() <= 0.02,
        "foreground ROI median expected ~1.20m, got {med}; stats={:?}",
        out.stats
    );

    // Saturated and flat textureless frames must return honest NaN, not hallucinated depth.
    let flat_l = Rgb::new(192, 128, vec![120u8; 192 * 128 * 3]);
    let flat_r = Rgb::new(192, 128, vec![120u8; 192 * 128 * 3]);
    let flat_out = sgbm(&flat_l, &flat_r, &rig, &cfg, None).unwrap();
    assert_eq!(flat_out.stats.valid, 0);
    assert!(flat_out.depth.data.iter().all(|d| !d.is_finite()));

    let blown = Rgb::new(192, 128, vec![255u8; 192 * 128 * 3]);
    let blown_out = sgbm(&blown, &blown, &rig, &cfg, None).unwrap();
    assert_eq!(blown_out.stats.valid, 0);
    assert!(blown_out.stats.saturated > 0);

    // Horizon far depth (> max_depth_m = 2.0) maps to INFINITY, not NaN or clamped 2.0!
    let mut cfg_short = cfg;
    cfg_short.max_depth_m = 2.0;
    let short_out = sgbm(&left, &right, &rig, &cfg_short, None).unwrap();
    let mut bg_inf = 0usize;
    let mut bg_finite = 0usize;
    let mut bg_total = 0usize;
    for y in 12..40 {
        for x in 20..260 {
            bg_total += 1;
            let d = short_out.depth.at(x, y);
            if d.is_infinite() {
                bg_inf += 1;
            } else if d.is_finite() {
                bg_finite += 1;
            }
        }
    }
    assert!(
        short_out.stats.far > 5000 && bg_inf * 100 >= 80 * bg_total && bg_finite == 0,
        "expected infinite background beyond max_depth_m=2.0: bg_inf={bg_inf}/{bg_total}, bg_finite={bg_finite}, stats={:?}",
        short_out.stats
    );

    // Monocular fallback guard: 0 bytes loaded when cameras >= 2!
    let mut loaded_flag = false;
    let res = maybe_load_monocular(2, || {
        loaded_flag = true;
        unreachable!("must not allocate TIPSv2 when stereo cameras >= 2");
    })
    .unwrap();
    assert!(res.is_none() && !loaded_flag);

    // StereoSpatialTier dispatch without SAM3.1
    let intr = Intrinsics { width: 384, height: 256, fov_deg: 70.0, pitch_deg: -10.0, roll_deg: 0.0 };
    let pose = Pose { x: 0.0, z: 0.0, yaw: 0.0, eye_m: 1.5 };
    let mut pipe = StereoSpatialPipeline::new(rig, cfg, None, None);
    let fast = pipe.run_tier(StereoSpatialTier::StereoFastTick, &left, &right, &intr, &pose).unwrap();
    assert_eq!(fast.tier, StereoSpatialTier::StereoFastTick);
    let grasp = pipe.run_tier(StereoSpatialTier::StereoMetricGrasp, &left, &right, &intr, &pose).unwrap();
    assert_eq!(grasp.tier, StereoSpatialTier::StereoMetricGrasp);
    let mut sd = SgbmDepth::new(rig, cfg);
    assert!(sd.infer(&left).is_err(), "SgbmDepth rejects monocular calls cleanly");
    assert!(sd.infer_pair(&left, &right).is_ok());
}

#[test]
fn gate_t1_spatial_b_zkey_bilateral_hole_recovery_and_unlimited_3d_tracking() {
    let intr = Intrinsics { width: 120, height: 80, fov_deg: 70.0, pitch_deg: 0.0, roll_deg: 0.0 };
    let pose = Pose { x: 0.0, z: 0.0, yaw: 0.0, eye_m: 1.5 };
    let (w, h) = (60u32, 40u32);

    // 30-frame static background capture at z = 6.0m with small noise
    let bg_frames_storage: Vec<DepthMap> = (0..30)
        .map(|k| DepthMap {
            width: w,
            height: h,
            kind: DepthKind::Metric,
            data: vec![6.0 + ((k % 3) as f32 - 1.0) * 0.02; (w * h) as usize],
        })
        .collect();
    let bg_refs: Vec<&DepthMap> = bg_frames_storage.iter().collect();
    let rgb_bg = Rgb::new(120, 80, vec![30u8; 120 * 80 * 3]);
    let bg = StageBackground::capture_median(&bg_refs, Some(rgb_bg), 0.03).unwrap();
    assert_eq!(bg.frame_count, 30);

    // Frame with 2 crossing performers wearing IDENTICAL white shirts ([230,230,230]) at distinct depths
    // Z1 = 2.0m (x in 15..25) and Z2 = 4.0m (x in 22..32), plus a NaN textureless hole on performer 1!
    let mut cur_depth = DepthMap { width: w, height: h, kind: DepthKind::Metric, data: vec![6.0; (w * h) as usize] };
    let mut cur_rgb_buf = vec![30u8; (120 * 80 * 3) as usize];
    for y in 8..32 {
        for x in 15..25 {
            cur_depth.data[(y * w + x) as usize] = 2.0;
            // Paint crisp high-res RGB pixels
            for ry in (y * 2)..(y * 2 + 2) {
                for rx in (x * 2)..(x * 2 + 2) {
                    let i = ((ry * 120 + rx) * 3) as usize;
                    cur_rgb_buf[i..i + 3].copy_from_slice(&[230, 230, 230]);
                }
            }
        }
        for x in 25..35 {
            cur_depth.data[(y * w + x) as usize] = 4.2;
            for ry in (y * 2)..(y * 2 + 2) {
                for rx in (x * 2)..(x * 2 + 2) {
                    let i = ((ry * 120 + rx) * 3) as usize;
                    cur_rgb_buf[i..i + 3].copy_from_slice(&[230, 230, 230]);
                }
            }
        }
    }
    // Inject textureless NaN holes inside performer 1 (Z1 = 2.0m) -> recovered by RGB motion delta!
    cur_depth.data[(15 * w + 19) as usize] = f32::NAN;
    cur_depth.data[(16 * w + 19) as usize] = f32::NAN;
    let cur_rgb = Rgb::new(120, 80, cur_rgb_buf);

    let mut tracker = StageZKeyTracker::new(bg.clone(), ZKeyCfg::default());
    let res = tracker.step(&cur_rgb, &cur_depth, &intr, &pose, None).unwrap();
    assert!(res.hole_recovered_px >= 2, "NaN hole inside moving performer must be recovered by RGB delta");
    assert_eq!(res.matte.width, 120);
    assert_eq!(res.matte.height, 80);
    assert_eq!(res.performers.len(), 2, "Identical shirt performers at Z=2.0m & Z=4.2m must NOT merge: {:?}", res.performers);
    let ids: BTreeSet<u32> = res.performers.iter().map(|p| p.id).collect();
    assert_eq!(ids.len(), 2);

    // Unlimited count (>16): simulate 20 spatially separated 3D clusters -> all 20 tracked!
    let wide_w = 200u32;
    let wide_h = 20u32;
    let bg20_d = DepthMap { width: wide_w, height: wide_h, kind: DepthKind::Metric, data: vec![8.0; (wide_w * wide_h) as usize] };
    let bg20 = StageBackground::capture_median(&[&bg20_d], None, 0.02).unwrap();
    let mut fg20_d = bg20_d.clone();
    for k in 0..20u32 {
        let x0 = k * 10 + 1;
        for y in 4..16 {
            for x in x0..(x0 + 4) {
                fg20_d.data[(y * wide_w + x) as usize] = 2.5;
            }
        }
    }
    let rgb20 = Rgb::new(200, 20, vec![120; 200 * 20 * 3]);
    let intr20 = Intrinsics { width: 200, height: 20, fov_deg: 90.0, pitch_deg: 0.0, roll_deg: 0.0 };
    let mut t20 = StageZKeyTracker::new(bg20, ZKeyCfg { border_fix: 0, hole_fill: 0, dot_remove: 0, min_cluster_px: 4, ..ZKeyCfg::default() });
    let out20 = t20.step(&rgb20, &fg20_d, &intr20, &pose, None).unwrap();
    assert_eq!(out20.performers.len(), 20, "Must track >= 20 performers with zero 16-object limit");
}

#[test]
fn gate_t1_spatial_c_tips_header_alignment_and_navigation_stack() {
    // 1. TIPSv2 full header tensor wiring (no candle-transformers, using antialias_bilinear_weights)
    assert_eq!(antialias_bilinear_weights(3, 3), vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    let raw = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tipsv2_b14_dpt_header.json"),
    )
    .unwrap();
    let header: BTreeMap<String, serde_json::Value> = serde_json::from_str(&raw).unwrap();
    let requested = Arc::new(Mutex::new(BTreeSet::new()));
    struct HBackend(BTreeMap<String, Vec<usize>>, Arc<Mutex<BTreeSet<String>>>);
    impl SimpleBackend for HBackend {
        fn get(&self, s: Shape, name: &str, _: candle_nn::Init, dtype: DType, dev: &Device) -> candle_core::Result<Tensor> {
            let shape = self.0.get(name).expect(name);
            assert_eq!(s.dims(), shape.as_slice(), "{name}");
            self.1.lock().unwrap().insert(name.to_string());
            Tensor::rand(-0.02f32, 0.02, shape.as_slice(), dev)?.to_dtype(dtype)
        }
        fn get_unchecked(&self, name: &str, dtype: DType, dev: &Device) -> candle_core::Result<Tensor> {
            let shape = self.0.get(name).expect(name);
            self.1.lock().unwrap().insert(name.to_string());
            Tensor::rand(-0.02f32, 0.02, shape.as_slice(), dev)?.to_dtype(dtype)
        }
        fn contains_tensor(&self, name: &str) -> bool { self.0.contains_key(name) }
    }
    let shapes: BTreeMap<String, Vec<usize>> = header
        .into_iter()
        .map(|(k, v)| (k, v["shape"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as usize).collect()))
        .collect();
    let vb = VarBuilder::from_backend(Box::new(HBackend(shapes, requested.clone())), DType::F32, Device::Cpu);
    let mut model = TipsDepth::from_var_builder_with(TipsConfig::default(), vb, &Device::Cpu, SceneHeads::ALL).unwrap();
    assert_eq!(requested.lock().unwrap().len(), 175 + 3 * 52);
    model.input_grid = InputGrid::Fixed { cols: 4, rows: 3 };
    let img = Rgb::new(56, 42, vec![100u8; 56 * 42 * 3]);
    let scene = model.infer_scene_batch(&[&img], SceneHeads::ALL).unwrap().pop().unwrap();
    assert_eq!(scene.depth.data.len(), 56 * 42);
    assert!(scene.normals.is_some());
    assert_eq!(scene.segmentation.as_ref().unwrap().labels.iter().filter(|&&l| (l as usize) < ADE_CLASSES).count(), scene.segmentation.unwrap().labels.len());

    // 2. Pyramid 1:2:4 + double_center locate + align + stairs + item + egoexo + fit3 + pos2 + decide
    let pyr = Pyramid::for_frame(384, 256, &PyramidCfg::default());
    assert_eq!((pyr.cols, pyr.rows), (18, 12));
    assert_eq!(pyr.l2(), (36, 24));
    assert_eq!(pyr.mask(), (576, 384));

    let intr = Intrinsics { width: 384, height: 256, fov_deg: 70.0, pitch_deg: -18.0, roll_deg: 0.0 };
    let pose = Pose { x: 0.0, z: 0.0, yaw: 0.0, eye_m: 1.5 };
    let mut gt_data = Vec::with_capacity(384 * 256);
    for y in 0..256 {
        for x in 0..384 {
            gt_data.push(ground_depth(&intr, &pose, x as f32 + 0.5, y as f32 + 0.5, None, 20.0).unwrap_or(f32::INFINITY));
        }
    }
    let mut pred = DepthMap {
        width: 384,
        height: 256,
        kind: DepthKind::Relative,
        data: gt_data.iter().map(|&d| if d.is_finite() { d * 0.65 } else { 15.0 }).collect(),
    };
    let fit = ground_align(&mut pred, &intr, &pose, None, Some(15.0), &AlignCfg { space: AlignSpace::Scale, ..AlignCfg::default() }).unwrap();
    assert!((fit.a - 1.0 / 0.65).abs() < 0.05);

    // Rigid fit3 check
    let src = [[1.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 3.0]];
    let dst = [[1.0, 1.0, 0.0], [0.0, 3.0, 0.0], [0.0, 1.0, 3.0]];
    let rigid = align_rigid(&src, &dst).unwrap();
    assert!((rigid.t[1] - 1.0).abs() < 1e-6);
    assert!(pa_mpjpe(&src, &dst).unwrap() < 1e-6);

    // Ego-Exo fusion collapses anisotropic Z error
    let (mean, cov) = fuse(&[
        ViewObs { origin: [0.0, 1.5, 0.0], point: [0.0, 1.0, 3.0], sigma_z: 0.3, sigma_theta: 0.005, weight: 1.0 },
        ViewObs { origin: [3.0, 1.5, 3.0], point: [0.05, 1.0, 3.0], sigma_z: 0.3, sigma_theta: 0.005, weight: 1.0 },
    ])
    .unwrap();
    assert!(cov[2][2] < 0.02 && (mean[2] - 3.0).abs() < 0.1);

    // Pos2 + Decider
    let mut f = Pos2Filter::default();
    for t in 0..8u64 {
        f.push(Pos2Obs { tick: t, x: 2.0 + 0.1 * t as f64, z: 1.0, sigma_m: 0.05 });
    }
    assert_eq!(f.model(), Pos2Model::Dynamic);
    let mut dec = Decider::new(DecideCfg::default());
    let rep = dec.step(&pose, &intr, Some(Sighting {
        keyframe: 1, pose, heading: 0.1, sigma: 0.01, extent: 0.0, depth_range_m: Some(2.5), track_id: 1,
    }), true);
    assert!(matches!(rep.decision, Decision::Approach { .. }));

    // Item pose + Gate points smoke
    let mut box_d = DepthMap { width: 64, height: 48, kind: DepthKind::Metric, data: vec![4.0; 64 * 48] };
    let ip = item_pose(&box_d, &intr, &pose, [0.3, 0.3, 0.7, 0.7], 0.0);
    assert!(ip.is_some());
    let _ = (read_step(&box_d, &intr, &pose, &StepCfg::default()), &mut box_d);
    let heats = label_heats(
        &vec![vec![0.5f32; 4]; 18 * 12],
        18,
        12,
        &LabelTexts { labels: vec![("a".into(), vec![0.5; 4], None), ("b".into(), vec![0.1; 4], None)], background: vec![] },
        &LocateCfg { scoring: LocateScoring::Cosine, double_center: true, ..LocateCfg::default() },
        10.0,
        -1.0,
    );
    let mut pts = lv_spatial::locate::points_with_presence(384, 256, heats);
    let geo = cell_geometry(&pyr, &pred, None, Some(View { intr: &intr, pose: &pose, height: None }), 18.0);
    let _ = gate_points(&mut pts, &pyr, &geo, &CellGateCfg::default());
    let _ = snap_points(&mut pts, &pyr, &pred, None, &SnapCfg::default());
    let _ = (Mask::empty(8, 8), fit_ground(&pred, &intr, &pose, None, Some(15.0), &AlignCfg::default()));
}
