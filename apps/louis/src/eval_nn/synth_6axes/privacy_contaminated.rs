//! Axis 5 (`PrivacyContaminated`): CounterKiosk lock, WAN reject, overlay WDA+EpochWatch wipe, and BIP-39 8-shard vault.

use anyhow::{ensure, Result};
use lv_core::residency::{ModelId, QwenTier};
use lv_core::siglip2::tower::{EpochWatch, VisionAbort};
use lv_core::siglip2::PatchGrid;
use lv_router::residency_router::ensure_role_allows_model;
use lv_router::vault::{DeviceRole, ProtectedKey, VaultError};
use lv_screen::REPRIME_THRESHOLD_MILLI;
use premium_discount_nav::{PrivacyShield, SurfaceSignature};

use crate::overlay::OverlayController;
use crate::vault_lan::{ensure_local_lan_addr, LouisVaultSession};

pub fn run_privacy_contaminated_d5_probe() -> Result<()> {
    // 1. CounterKiosk allowed ONLY Siglip2 & Nano08B; Base2B / Heavy4B / Gemma4 / unlock blocked.
    ensure!(ensure_role_allows_model(DeviceRole::CounterKiosk, ModelId::Siglip2).is_ok());
    ensure!(
        ensure_role_allows_model(DeviceRole::CounterKiosk, ModelId::Qwen35(QwenTier::Nano08B))
            .is_ok()
    );
    for forbidden in [
        ModelId::Gemma4,
        ModelId::Qwen35(QwenTier::Base2B),
        ModelId::Qwen35(QwenTier::Heavy4B),
    ] {
        ensure!(ensure_role_allows_model(DeviceRole::CounterKiosk, forbidden) == Err(VaultError::KioskForbidden));
    }
    let mut kiosk = LouisVaultSession::new(DeviceRole::CounterKiosk, None)?;
    ensure!(
        kiosk.unlock(DeviceRole::CounterKiosk, ProtectedKey::generate())
            == Err(VaultError::KioskForbidden)
    );
    ensure!(ensure_local_lan_addr("8.8.8.8:9999".parse()?).is_err());

    // 2. Desktop overlay lockdown + WDA 0x11 + click-through 0x080800A8 + secret zeroize
    let mut ov = OverlayController::new(64, 64, 0.15);
    ensure!(ov.self_display_affinity() == 0x11);
    ensure!(ov.window_ex_style() == 0x0808_00A8);
    let bgra = vec![90u8; 64 * 64 * 4];
    let mut secret_rgb = vec![231u8; 64 * 64 * 3];
    let tick = ov.step_frame(
        100,
        "bank.exe",
        SurfaceSignature::new(9, "bank.exe"),
        &[],
        &[],
        true,
        true,
        &mut secret_rgb,
        &bgra,
        false,
    );
    ensure!(!tick.capture_allowed && matches!(tick.shield, PrivacyShield::FullLockdown(_)));
    ensure!(secret_rgb.iter().all(|&b| b == 0));
    ensure!(REPRIME_THRESHOLD_MILLI == u32::MAX);

    // 3. EpochWatch mid-encode abort + PatchGrid wipe
    let epoch = ov.epoch_handle();
    let watch = EpochWatch::capture(&epoch);
    let mut grid = PatchGrid {
        grid_h: 2,
        grid_w: 2,
        patches: vec![1.0f32; 4 * PatchGrid::DIM],
    };
    ov.notify_external_privacy_hit();
    ensure!(watch.check() == Err(VisionAbort::PrivacyLocked));
    grid.wipe();
    ensure!(grid.patches.iter().all(|&x| x == 0.0));

    // 4. Local X25519 vault seal + open + grace zeroize
    let mut phone = LouisVaultSession::new(DeviceRole::PersonalPhone, None)?;
    phone.pair_over_local_tcp("127.0.0.1:0".parse()?, DeviceRole::PersonalPhone, b"d5-probe")?;
    let sealed = phone.seal_record(b"aad", b"sensitive-payload")?;
    ensure!(phone.open_record(b"aad", &sealed)? == b"sensitive-payload");
    phone.disconnect_grace_blur();
    phone.expire_grace_and_zeroize();
    Ok(())
}
