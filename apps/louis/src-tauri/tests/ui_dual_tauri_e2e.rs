//! Layer 3 (`Step UI-C`) Integration & `--release` (`panic = "abort"`) Survivability E2E
//! for Dual-App Anchor (`louis-desktop` <-> `louis-vision-app`), GPU Lease Handoff,
//! Win32 Overlay Capture Shield, 2D+3D Privacy Zeroize, 5-Layer SQL Guard, and Sidecar Probes.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use louis_desktop_lib::router_host::{
    ensure_loopback_bind, find_vision_peer, overlay_ex_style_for, route_intent,
    ActiveSurfaceOwner, GpuLeaseTier, HostRouterState, LoopbackWireMsg, N2sfWireGrade,
    WarmLeaseKind, DESKTOP_ANCHOR_LOCK, ROUTER_LOOPBACK_PORT, SPECULATIVE_TTL_SECS,
    STAGE_VRAM_CAP_MB, VISION_MCP_PORT,
};
use louis_desktop_lib::sql_guard::{validate_and_wrap_sql, SqlGuardError};
use louis_desktop_lib::ui_nn_defs::{ui_defs_rev, UI_NN_DIAG_SEED, UI_NN_HOLDOUT_SEED};
use louis_desktop_lib::{check_field3d_role, run_reverse_cdr_guard};
use louis_docs::overlay::OverlayController;
use louis_docs::run_selftest;
use lv_agent::gates::{check_manual_amount, select_form_for_verdict, AgentVerdict, OutputForm};
use lv_core::device::{
    probe_current_process_accel, reset_accel_cache, set_accel_fault, trusted_with_cache, AccelOp,
    DevicePref, DeviceVramSnapshot,
};
use lv_doc::writer::guard::N2sfGrade;
use lv_domain::EngineMode;
use lv_router::vault::DeviceRole;
use lv_screen::REPRIME_THRESHOLD_MILLI;
use premium_discount_eval::accel_tol::ToleranceTier;

fn sidecar_probe_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join("lv_accel_probe-x86_64-pc-windows-msvc.exe")
}

fn sidecar_vision_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join("louis-vision-app-x86_64-pc-windows-msvc.exe")
}

#[test]
fn layer3_single_anchor_gpu_lease_overlay_sql_and_gates() {
    assert_eq!(DESKTOP_ANCHOR_LOCK, "louis.desktop.anchor.v1");
    assert_eq!(ROUTER_LOOPBACK_PORT, 8787);
    assert_eq!(VISION_MCP_PORT, 8788);
    assert_eq!(SPECULATIVE_TTL_SECS, 45);
    assert_eq!(UI_NN_DIAG_SEED, 2026_0501);
    assert_eq!(UI_NN_HOLDOUT_SEED, 2026_1126);

    let rev = ui_defs_rev();
    assert_eq!(rev.len(), 8);
    assert!(rev.chars().all(|c| c.is_ascii_hexdigit()));

    // Loopback control plane rejects public WAN IP.
    assert!(ensure_loopback_bind("127.0.0.1:8787".parse().unwrap()).is_ok());
    assert!(ensure_loopback_bind("8.8.8.8:8787".parse().unwrap()).is_err());

    // Win32 LayerFlags masks & WDA_EXCLUDEFROMCAPTURE.
    assert_eq!(overlay_ex_style_for(true, true, false), 0x0808_00A8);
    assert_eq!(overlay_ex_style_for(false, true, false), 0x0808_0088);
    assert_eq!(overlay_ex_style_for(false, true, true), 0x0008_0088);

    // Dual-app single star anchor & GPU lease transitions.
    let mut host = HostRouterState::default();
    assert_eq!(host.active_star_count(), 1);
    let spec = route_intent("리허설 최대 오차", &[], DeviceRole::PersonalPhone);
    assert_eq!(spec.warm_lease, WarmLeaseKind::Speculative { ttl_secs: 45 });

    let _hb = host.apply_wire(LoopbackWireMsg::Handshake {
        pid: 1001,
        role: DeviceRole::PersonalPhone,
        n2sf: N2sfWireGrade::Sensitive,
        owner: ActiveSurfaceOwner::VisionSimSandbox,
    });
    assert_eq!(host.active_star_count(), 1);
    assert!(!host.louis_overlay_visible);
    assert!(host.vision_star_visible);

    let ack = host.apply_wire(LoopbackWireMsg::RequestGpuLease {
        target: GpuLeaseTier::StagePinnedLock,
        requester_pid: 2002,
    });
    assert!(matches!(
        ack,
        LoopbackWireMsg::AckGpuLease {
            granted: GpuLeaseTier::StagePinnedLock,
            louis_resident_mb,
        } if louis_resident_mb <= STAGE_VRAM_CAP_MB
    ));
    assert!(host.ingest_disk_queue_only);
    assert_eq!(host.active_star_count(), 1);

    let snap_locked = DeviceVramSnapshot {
        driver_free_bytes: 8 * 1024 * 1024 * 1024,
        os_budget_free_bytes: 8 * 1024 * 1024 * 1024,
        pool_idle_bytes: 0,
        proc_active_bytes: Some(0),
        total_bytes: 8 * 1024 * 1024 * 1024,
    };
    assert!(snap_locked.resident_used_mb_f32() <= STAGE_VRAM_CAP_MB);

    let bc = host.apply_wire(LoopbackWireMsg::BroadcastPrivacyEpoch {
        epoch: 1,
        locked: true,
        reason: "bank.exe".into(),
    });
    assert!(matches!(
        bc,
        LoopbackWireMsg::BroadcastPrivacyEpoch { epoch, locked: true, .. } if epoch >= 1
    ));

    // Full 2D overlay self-test (WDA_EXCLUDEFROMCAPTURE 0x11, 0x0808_00A8, bank.exe RGB zeroize, REPRIME, LAN, 7 packs).
    let summary = run_selftest(EngineMode::RulesOnly).expect("louis-docs selftest must pass");
    assert!(summary.contains("counterexamples=6/6"));
    let mut ov = OverlayController::new(64, 64, 0.15);
    assert_eq!(ov.self_display_affinity(), 0x11);
    assert!(ov.notify_external_privacy_hit() >= 1);
    assert_eq!(OverlayController::reprime_sentinel(), REPRIME_THRESHOLD_MILLI);

    // 5-Layer DataFusion SQL guard.
    assert!(validate_and_wrap_sql(
        "DROP TABLE docs_entities",
        DeviceRole::PersonalPhone,
        "vault-default",
        1
    )
    .is_err());
    assert!(validate_and_wrap_sql(
        "SELECT read_file('/etc/passwd')",
        DeviceRole::PersonalPhone,
        "vault-default",
        1
    )
    .is_err());
    assert_eq!(
        validate_and_wrap_sql(
            "SELECT doc_id FROM docs_entities",
            DeviceRole::CounterKiosk,
            "vault-default",
            0
        ),
        Err(SqlGuardError::KioskForbidden)
    );
    let plan = validate_and_wrap_sql(
        "SELECT doc_id, amt_krw FROM docs_entities",
        DeviceRole::PersonalPhone,
        "vault-default",
        1,
    )
    .expect("valid SELECT must pass");
    assert!(plan.wrapped_sql.contains("vault-default"));
    assert!(plan.effective_limit <= 200);

    // 5-Gate form override, +1 KRW block, Reverse-CDR, CounterKiosk Field-3D block.
    let snapped = select_form_for_verdict(
        OutputForm::ApprovalNotice,
        AgentVerdict::G2RejectContradictionLock,
        0,
        false,
    );
    assert_eq!(snapped.form, OutputForm::RejectNotice);
    assert!(snapped.overridden);
    assert_eq!(
        check_manual_amount(80_000_000, 85_000_001, 5_000_000),
        Err(AgentVerdict::G2BlockUnprovenEdit)
    );
    assert!(run_reverse_cdr_guard(
        "기밀 유출 시도",
        N2sfGrade::Classified,
        N2sfGrade::Open,
        Some("DECL-1".into()),
        0,
        true
    )
    .is_err());
    assert!(run_reverse_cdr_guard(
        "순수 XML",
        N2sfGrade::Sensitive,
        N2sfGrade::Open,
        Some("N2SF-2026-0812".into()),
        0,
        true
    )
    .is_ok());
    assert!(check_field3d_role(DeviceRole::CounterKiosk).is_err());
}

#[cfg(feature = "field-3d")]
#[test]
fn layer3_3d_display_screen_fail_closed_zeroize() {
    use lv_planar::homography::{compute_planar_homography, unwarp_planar_patch};
    use lv_planar::screen_bridge::{
        inspect_and_guard_3d_screen, PlanarScreenGuardCfg, PlanarScreenOutcome, ScreenRedactReason,
    };
    use lv_spatial::camera::{Intrinsics, Pose};
    use lv_spatial::fit3::V3;
    use lv_spatial::frame::Rgb;
    use lv_spatial::planar::{PhysicalFormFactor, PlanarPatch3D, SurfaceClassHint};
    use lv_spatial::egoexo::ViewObs;

    let intr = Intrinsics::pinhole(320, 240, 22.0);
    let mut pose = Pose::at_pos(0.0, 1.5, 0.0);
    pose.yaw = std::f32::consts::FRAC_PI_2;

    let patch = PlanarPatch3D {
        center_w: [0.0, 1.5, 0.50],
        normal_w: [0.0, 0.0, -1.0],
        tangent_u_w: [-1.0, 0.0, 0.0],
        tangent_v_w: [0.0, -1.0, 0.0],
        width_mm: 120.0,
        height_mm: 70.0,
        form_factor: PhysicalFormFactor::Display16x9,
        hint: SurfaceClassHint::DisplayScreen,
        corners_w: [
            [-0.06, 1.535, 0.50],
            [0.06, 1.535, 0.50],
            [0.06, 1.465, 0.50],
            [-0.06, 1.465, 0.50],
        ],
        corners_px: [[90.0, 70.0], [230.0, 70.0], [230.0, 170.0], [90.0, 170.0]],
        effective_px_per_mm_u: 3.0,
        effective_px_per_mm_v: 3.0,
        view_cos: 1.0,
        depth_hole_frac: 0.0,
        plane_rmse_m: 0.0,
        sigma_z_m: 0.002,
        inliers: 100,
        view_obs: ViewObs {
            origin: [0.0, 1.5, 0.0] as V3,
            point: [0.0, 1.5, 0.50] as V3,
            sigma_z: 0.002,
            sigma_theta: 0.002,
            weight: 4.0,
        },
    };
    let hom = compute_planar_homography(&patch, &intr, &pose, 2.5, 0.0, 512).unwrap();
    let mut rgb = Rgb::new(320, 240, vec![210u8; 320 * 240 * 3]);
    let mut rect = unwarp_planar_patch(&rgb, hom);

    let epoch = Arc::new(AtomicU64::new(1));
    let outcome = inspect_and_guard_3d_screen(
        &mut rgb,
        &mut rect,
        &patch,
        None,
        &epoch,
        0,
        &PlanarScreenGuardCfg::default(),
    )
    .unwrap();
    match outcome {
        PlanarScreenOutcome::PrivacyRedacted {
            zeroized_pixels,
            reason,
            trigger_sensor_purge,
        } => {
            assert!(zeroized_pixels > 5000);
            assert_eq!(reason, ScreenRedactReason::EpochChangedDuringPass);
            assert!(trigger_sensor_purge);
        }
        other => panic!("expected PrivacyRedacted, got {other:?}"),
    }
    assert!(rect.rgb.iter().all(|&b| b == 0));
    let center_idx = (120 * 320 + 160) * 3;
    assert_eq!(&rgb.data[center_idx..center_idx + 3], &[0, 0, 0]);
    assert_eq!(epoch.load(Ordering::Acquire), 1);
}

#[test]
fn layer3_sidecars_and_panic_abort_fault_injection() {
    let probe = sidecar_probe_path();
    assert!(probe.is_file(), "missing staged lv_accel_probe at {}", probe.display());
    let vision_peer = sidecar_vision_path();
    assert!(vision_peer.is_file(), "missing staged louis-vision-app at {}", vision_peer.display());

    // Execute isolated `--lv-accel-probe --backend cpu --ordinal 0` child process.
    let out = Command::new(&probe)
        .args(["--lv-accel-probe", "--backend", "cpu", "--ordinal", "0"])
        .output()
        .expect("spawn staged lv_accel_probe sidecar");
    assert!(
        out.status.success(),
        "lv_accel_probe failed ({}): {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let json_text = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(json_text.trim()).expect("probe stdout must be valid JSON");
    assert_eq!(parsed["panic_abort_safe"], true);
    assert_eq!(parsed["hardware_verified"], true);

    // Verify `find_vision_peer` resolves sidecar adjacent to executable.
    let tmp_dir = std::env::temp_dir().join("lv_peer_find_test");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let dummy_host = tmp_dir.join("louis-desktop.exe");
    let dummy_peer = tmp_dir.join("louis-vision-app.exe");
    std::fs::copy(&vision_peer, &dummy_peer).expect("copy vision peer into sibling test dir");
    let found = find_vision_peer(None, Some(&dummy_host)).expect("find_vision_peer must locate sibling exe");
    assert_eq!(found.file_name().and_then(|s| s.to_str()), Some("louis-vision-app.exe"));
    let _ = std::fs::remove_dir_all(&tmp_dir);

    // Under `--release` (`panic = "abort"`), fault injection must degrade gracefully with 0 panics / 0 SIGABRT.
    assert!(DevicePref::parse("nonexistent_gpu").is_err());
    reset_accel_cache();
    set_accel_fault(AccelOp::DecoderOps, true);
    let ok = trusted_with_cache("e2e:0", AccelOp::DecoderOps, ToleranceTier::F32Compute, || {
        Ok((vec![1.0, 2.0], vec![1.0, 2.0]))
    });
    assert!(!ok);
    reset_accel_cache();

    std::env::set_var("LV_PROBE_ADAPTER_NAME", "llvmpipe");
    let soft_vk = probe_current_process_accel("vulkan", 0);
    std::env::remove_var("LV_PROBE_ADAPTER_NAME");
    assert!(!soft_vk.hardware_verified);
    assert!(soft_vk.panic_abort_safe);
}
