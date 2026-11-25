//! Tier 2-B1 air-gapped office automation application (`louis-office`).

pub mod overlay;
pub mod vault_lan;
pub mod watcher;
pub mod workflow;

use anyhow::Result;
use lv_core::residency::{Group, ModelId, Phase};
use lv_domain::EngineMode;
use lv_router::vault::{DeviceRole, ProtectedKey, VaultError};
use lv_screen::REPRIME_THRESHOLD_MILLI;
use overlay::OverlayController;
use premium_discount_nav::{PrivacyShield, SurfaceSignature};
use vault_lan::{ensure_local_lan_addr, OfficeVaultSession};
use watcher::StagedOfficePipeline;
use workflow::evaluate_all_packs;

/// Executes an end-to-end offline self-test verifying overlay redaction/zeroize,
/// LAN X25519 vault isolation, staged inference locks, and all 7 domain packs.
pub fn run_selftest(mode: EngineMode) -> Result<String> {
    let mut ov = OverlayController::new(64, 64, 0.15);
    anyhow::ensure!(ov.self_display_affinity() == 0x11);
    anyhow::ensure!(ov.window_ex_style() == 0x0808_00A8);
    anyhow::ensure!(ov.effective_alpha(true) <= 76);

    let bgra = vec![96u8; 64 * 64 * 4];
    let mut rgb = vec![96u8; 64 * 64 * 3];
    let t0 = ov.step_frame(0, "hwp.exe", SurfaceSignature::new(1, "hwp.exe"), &[], &[], false, false, &mut rgb, &bgra, true);
    anyhow::ensure!(t0.capture_allowed && t0.threshold_milli == REPRIME_THRESHOLD_MILLI);

    let mut secret_rgb = vec![220u8; 64 * 64 * 3];
    let t_priv = ov.step_frame(500, "bank.exe", SurfaceSignature::new(2, "bank.exe"), &[], &[], true, true, &mut secret_rgb, &bgra, false);
    anyhow::ensure!(!t_priv.capture_allowed && matches!(t_priv.shield, PrivacyShield::FullLockdown(_)));
    anyhow::ensure!(secret_rgb.iter().all(|&b| b == 0));

    let wan: std::net::SocketAddr = "1.1.1.1:9999".parse()?;
    anyhow::ensure!(ensure_local_lan_addr(wan).is_err());

    let mut kiosk = OfficeVaultSession::new(DeviceRole::CounterKiosk, None)?;
    anyhow::ensure!(kiosk.unlock(DeviceRole::CounterKiosk, ProtectedKey::generate()) == Err(VaultError::KioskForbidden));

    let mut phone = OfficeVaultSession::new(DeviceRole::PersonalPhone, None)?;
    phone.pair_over_local_tcp("127.0.0.1:0".parse()?, DeviceRole::PersonalPhone, b"selftest")?;
    let blob = phone.seal_record(b"k", b"v")?;
    anyhow::ensure!(phone.open_record(b"k", &blob)? == b"v");
    phone.disconnect_grace_blur();
    phone.expire_grace_and_zeroize();

    let pipe = StagedOfficePipeline::new(4096);
    pipe.pin_slot(ModelId::Siglip2, Group::Vision, 512)?;
    anyhow::ensure!(pipe.lookup_readonly(ModelId::Siglip2, &Phase::new("ov", [Group::Vision]))? == 1);

    let suite = evaluate_all_packs(mode);
    anyhow::ensure!(suite.total_packs == 7 && suite.passed_packs == 7 && suite.gate_ab.passed);

    Ok(format!(
        "[LOUIS-OFFICE] selftest PASS mode={:?} packs={}/{} mean_cer={:.4} ex_style=0x{:08X} affinity=0x{:02X}",
        suite.mode,
        suite.passed_packs,
        suite.total_packs,
        suite.mean_cer,
        ov.window_ex_style(),
        ov.self_display_affinity()
    ))
}
