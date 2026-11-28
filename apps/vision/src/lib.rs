//! Tier 2-B2 60fps concert/broadcast XR live engine (`vision-sandbox`).
//! Zero SAM 3.1 / MobileSAM / `lv-doc` / `lv-screen` / `lv-agent` / `lancedb` / WAN HTTP crates.

pub mod calibration;
pub mod dmx_engine;
pub mod interlock;
pub mod jelly_stage;
pub mod live_loop;
pub mod mask_bridge;
pub mod rig_capture;
pub mod sandbox_stereo;
pub mod show_io;
pub mod stage_render;

use std::net::{IpAddr, SocketAddr};

use anyhow::{bail, ensure, Result};
use lv_spatial::camera::{Intrinsics, Pose};
use lv_spatial::egoexo::{stereo_sigma_z, ViewObs};
use lv_spatial::fit3::V3;
use lv_spatial::frame::{DepthKind, DepthMap, Rgb};
use lv_spatial::stereo::{sgbm, SgbmCfg, StereoRig};

use calibration::SandboxCalibration;
use interlock::{CueTriggerSource, InterlockDecision, InterlockInputs, SandboxHazardObb, SandboxInterlock};
use live_loop::{fuse_multi_view_tracks, SandboxLiveEngine};
use rig_capture::{decode_sbs_stereo_pair, RigSource, TallyState, WirelessCameraRig};
use sandbox_stereo::StereoSandboxLive;
use show_io::{
    build_zero_copy_descriptor, map_to_openxr_sandbox_space, pack_ndi_video_alpha,
    GpuShareBackend, OscUdpBridge, DEFAULT_OSC_ADDR,
};

/// Returns `true` only for loopback or private RFC1918 LAN addresses (blocks all WAN IPs).
pub fn is_local_or_rfc1918(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() || v4.is_private() || v4.is_link_local() {
                return true;
            }
            false
        }
        IpAddr::V6(v6) => {
            v6.is_loopback() || ((v6.segments()[0] & 0xfe00) == 0xfc00)
        }
    }
}

/// Validates that `addr` resolves to a local loopback or RFC1918 private LAN host.
pub fn validate_local_socket(addr: SocketAddr, label: &str) -> Result<SocketAddr> {
    if !is_local_or_rfc1918(&addr.ip()) {
        bail!("{label} rejects non-local WAN address {addr}");
    }
    Ok(addr)
}

fn smooth_nonperiodic_texture(w: u32, h: u32, seed: u64) -> Vec<f32> {
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
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let i = (y * w + x) as usize;
            nxt[i] = 0.5 * raw[i]
                + 0.125
                    * (raw[i - 1] + raw[i + 1] + raw[i - w as usize] + raw[i + w as usize]);
        }
    }
    nxt
}

fn sample_tex_f32(tex: &[f32], w: u32, x: f32, y: u32) -> f32 {
    let xc = x.clamp(0.0, (w - 1) as f32);
    let x0 = xc.floor() as u32;
    let x1 = (x0 + 1).min(w - 1);
    let t = xc - x0 as f32;
    tex[(y * w + x0) as usize] * (1.0 - t) + tex[(y * w + x1) as usize] * t
}

/// Builds a synthetic textured side-by-side stereo pair (`2 * w_eye` x `h`) with non-periodic
/// splitmix64 patterns so Census-SGBM matching achieves >= 85% valid stereo coverage cleanly.
pub fn synthetic_textured_sbs(w_eye: u32, h: u32, fg: Option<[u32; 4]>, bg_sh: i32, fg_sh: i32) -> Rgb {
    let tex_w = w_eye + bg_sh.max(fg_sh).max(0) as u32 + 16;
    let fg_tex = smooth_nonperiodic_texture(tex_w, h, 7);
    let bg_tex = smooth_nonperiodic_texture(tex_w, h, 1);
    let in_fg = |x: f32, y: u32| -> bool {
        fg.map_or(false, |b| x >= b[0] as f32 && x < b[2] as f32 && y >= b[1] && y < b[3])
    };
    let full_w = w_eye * 2;
    let mut data = vec![0u8; (full_w * h * 3) as usize];
    for y in 0..h {
        for x in 0..w_eye {
            let vl = if in_fg(x as f32, y) {
                fg_tex[(y * tex_w + x) as usize]
            } else {
                bg_tex[(y * tex_w + x) as usize]
            };
            let ul = vl.round().clamp(0.0, 240.0) as u8;
            let xf_fg = x as f32 + fg_sh as f32;
            let vr = if in_fg(xf_fg, y) {
                sample_tex_f32(&fg_tex, tex_w, xf_fg, y)
            } else {
                sample_tex_f32(&bg_tex, tex_w, x as f32 + bg_sh as f32, y)
            };
            let ur = vr.round().clamp(0.0, 240.0) as u8;
            let il = ((y * full_w + x) * 3) as usize;
            let ir = ((y * full_w + x + w_eye) * 3) as usize;
            data[il..il + 3].copy_from_slice(&[ul, ul, ul]);
            data[ir..ir + 3].copy_from_slice(&[ur, ur, ur]);
        }
    }
    Rgb::new(full_w, h, data)
}

/// Runs an end-to-end offline self-test covering rig capture, 30-frame SE(3) calibration,
/// SGBM + Z-Keying, pinned residency + `egoexo::fuse`, 3-step `SandboxInterlock`, and loopback OSC UDP.
pub fn run_selftest(osc_target: Option<SocketAddr>) -> Result<String> {
    let target = osc_target.unwrap_or_else(|| DEFAULT_OSC_ADDR.parse().unwrap());
    validate_local_socket(target, "CLI OSC target")?;

    let src_lan = RigSource::WirelessLanSync {
        quality: 96.5,
        endpoint: "192.168.10.20:7001".parse()?,
    };
    let mut rig_ui = WirelessCameraRig::new("cam-foh-1", src_lan.clone())?;
    ensure!(rig_ui.touch_locked && rig_ui.tally_red_luma_pct == 5);
    rig_ui.set_tally(TallyState::LiveOnAir, 5)?;

    let rig = StereoRig { focal_px: 300.0, baseline_m: 0.12, subpixel_px: 0.15 };
    let mut sgbm_cfg = SgbmCfg::default();
    sgbm_cfg.per_l2 = 8;
    sgbm_cfg.speckle_size = 8;

    let bg_sbs = synthetic_textured_sbs(288, 192, None, 6, 6);
    ensure!(decode_sbs_stereo_pair(&bg_sbs, 94.0, &src_lan).is_err());
    let (bg_l, bg_r) = decode_sbs_stereo_pair(&bg_sbs, 96.5, &src_lan)?;
    let bg_stereo = sgbm(&bg_l, &bg_r, &rig, &sgbm_cfg, None)?;
    let bg_frames: Vec<&DepthMap> = (0..30).map(|_| &bg_stereo.depth).collect();

    let rig_markers: Vec<V3> = vec![[0.0, 0.0, 2.0], [1.0, 0.0, 2.0], [0.0, 1.0, 2.0]];
    let cal = SandboxCalibration::calibrate_rigid(&rig_markers, &rig_markers, &bg_frames, Some(bg_l), 0.03)?;
    ensure!((cal.rig_to_sandbox.s - 1.0).abs() < 1e-9 && cal.bg.frame_count == 30);

    let intr = Intrinsics { width: 288, height: 192, fov_deg: 70.0, pitch_deg: 0.0, roll_deg: 0.0 };
    let pose = Pose { x: 0.0, z: 0.0, yaw: 0.0, eye_m: 1.5 };
    let sandbox = StereoSandboxLive::new(
        rig,
        sgbm_cfg,
        None,
        cal.bg,
        Default::default(),
        intr,
        pose,
        cal.rig_to_sandbox,
    );
    let mut engine = SandboxLiveEngine::new(64 * 1024 * 1024, sandbox)?;

    let fg_sbs = synthetic_textured_sbs(288, 192, Some([112, 48, 176, 152]), 6, 14);
    let (fg_l, fg_r) = decode_sbs_stereo_pair(&fg_sbs, 96.5, &src_lan)?;
    let (frame, budget) = engine.tick_stereo_pair(&fg_l, &fg_r)?;
    ensure!(engine.eviction_count() == 0 && !frame.performers.is_empty());

    let sz = stereo_sigma_z(2.5, 300.0, 0.12, 0.15);
    let fused = fuse_multi_view_tracks(&[
        ViewObs { origin: [-2.0, 1.5, 0.0], point: [0.002, 1.100, 2.502], sigma_z: sz, sigma_theta: 0.002, weight: 1.0 },
        ViewObs { origin: [2.0, 1.5, 0.0], point: [-0.002, 1.099, 2.499], sigma_z: sz, sigma_theta: 0.002, weight: 1.0 },
    ])
    .ok_or_else(|| anyhow::anyhow!("egoexo fuse failed"))?;
    ensure!(fused.within_20mm);

    let mut interlock = SandboxInterlock::new_memory(
        vec![SandboxHazardObb { name: "pyro_front".into(), min_xyz: [-1.0, 0.0, 0.2], max_xyz: [1.0, 2.5, 0.8] }],
        64,
    );
    let inputs = InterlockInputs {
        active_view_valid_ratios: vec![frame.stereo_valid_ratio, 0.92],
        metric_points_m: frame.performers.iter().map(|p| p.pos_m).collect(),
        fusion_residual_m: Some(fused.residual_m),
        residual_history_m: vec![0.005; 16],
        residual_sds_sd_m: Some(0.003),
    };
    let dispatch = interlock.dispatch_cue(CueTriggerSource::AutoTimecode, "cue_main", &inputs)?;
    ensure!(matches!(dispatch.decision, InterlockDecision::Clear { .. }));

    // Also prove loop-back OSC, GPU descriptor, NDI packet, and OpenXR anchor build cleanly.
    let rx = std::net::UdpSocket::bind("127.0.0.1:0")?;
    let osc = OscUdpBridge::bind(rx.local_addr()?)?;
    osc.send_track_xyz(frame.performers[0].id, frame.performers[0].pos_m)?;
    let _gpu = build_zero_copy_descriptor(
        GpuShareBackend::default_for_host(),
        "VisionSandbox_ID",
        &frame.matte,
        &frame.id_matte_rgba,
        &frame.depth,
        frame.frame_index,
    )?;
    let _ndi = pack_ndi_video_alpha("SandboxMain", "192.168.1.99:5961".parse()?, &fg_l, &frame.matte)?;
    let xr = map_to_openxr_sandbox_space(frame.frame_index, &frame.performers);

    let _dummy_depth = DepthMap { width: 2, height: 2, kind: DepthKind::Metric, data: vec![2.0; 4] };

    Ok(format!(
        "[VISION-SANDBOX] selftest PASS frame={} performers={} valid_ratio={:.3} fusion_res_mm={:.2} evictions={} xr_anchors={} budget_us={}",
        frame.frame_index,
        frame.performers.len(),
        frame.stereo_valid_ratio,
        fused.residual_m * 1000.0,
        engine.eviction_count(),
        xr.anchors.len(),
        budget.elapsed_us
    ))
}
