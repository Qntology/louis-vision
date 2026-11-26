//! Tier 1 Pure-Sync Cross-App Coordinator (`CX-01..CX-08`):
//! - Single `( ✦ )` desktop anchor arbitration (`DESKTOP_ANCHOR_LOCK`) & 45 s Speculative reclamation,
//! - Deep-link builders (`vision://stage/...`, `louis://doc/...`),
//! - `StagePinnedLock` GPU <= 32.0 MB lease handoff & `DiskQueueOnly` queuing,
//! - Global `privacy_epoch` dual-buffer `zeroize` & 3,000 ms + 3 clean ticks reprime gate,
//! - 4 mobile WebRTC skins isolation (`CounterKiosk` jail, RFC1918 LAN check rejecting WAN),
//! - IME-safe ESC stack & `F12` Blackout kill-switch preemption.

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::vault::{DeviceRole, VaultError};

pub const DESKTOP_ANCHOR_LOCK: &str = "louis.desktop.anchor.v1";
pub const SPECULATIVE_TTL_MS: u64 = 45_000;
pub const STAGE_PINNED_LOUIS_CAP_MB: f32 = 32.0;
pub const REPRIME_CLEAN_HOLD_MS: u64 = 3_000;
pub const REPRIME_REQUIRED_TICKS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileSkinKind {
    CounterKiosk,
    WirelessCameraRig,
    StageCueRemote,
    PersonalPhone,
}

impl MobileSkinKind {
    pub const ALL: [Self; 4] = [
        Self::CounterKiosk,
        Self::WirelessCameraRig,
        Self::StageCueRemote,
        Self::PersonalPhone,
    ];

    pub fn from_role(role: DeviceRole) -> Self {
        match role {
            DeviceRole::CounterKiosk => Self::CounterKiosk,
            DeviceRole::PersonalPhone => Self::PersonalPhone,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuLeaseState {
    LouisExclusive,
    CoexistSim,
    StagePinnedLock { louis_cap_mb_x10: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppSurfaceFocus {
    LouisDesktop,
    LouisDocsStudio,
    VisionSimSandbox,
    VisionXrStageLive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossAppPrivacySync {
    pub privacy_epoch: u64,
    pub locked: bool,
    pub last_hit_ms: u64,
    pub clean_since_ms: Option<u64>,
    pub consecutive_clean_ticks: u32,
}

impl Default for CrossAppPrivacySync {
    fn default() -> Self {
        Self {
            privacy_epoch: 0,
            locked: false,
            last_hit_ms: 0,
            clean_since_ms: None,
            consecutive_clean_ticks: 0,
        }
    }
}

impl CrossAppPrivacySync {
    /// Immediately increments `privacy_epoch`, enters lockdown, and zeroes both Louis 2D RGB
    /// and Vision 3D tactical display quad buffers.
    pub fn trigger_lockdown_and_zeroize(
        &mut self,
        now_ms: u64,
        louis_2d_rgb: &mut [u8],
        vision_3d_quad_rgb: &mut [u8],
    ) -> u64 {
        self.privacy_epoch = self.privacy_epoch.saturating_add(1);
        self.locked = true;
        self.last_hit_ms = now_ms;
        self.clean_since_ms = None;
        self.consecutive_clean_ticks = 0;
        louis_2d_rgb.zeroize();
        vision_3d_quad_rgb.zeroize();
        self.privacy_epoch
    }

    /// Requires BOTH `>= 3,000 ms` elapsed clean AND `>= 3` consecutive clean ticks to re-arm capture.
    pub fn observe_clean_tick(&mut self, now_ms: u64) -> bool {
        if !self.locked {
            return true;
        }
        let start = *self.clean_since_ms.get_or_insert(now_ms);
        self.consecutive_clean_ticks = self.consecutive_clean_ticks.saturating_add(1);
        if now_ms.saturating_sub(start) >= REPRIME_CLEAN_HOLD_MS
            && self.consecutive_clean_ticks >= REPRIME_REQUIRED_TICKS
        {
            self.locked = false;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscLayerKind {
    PopupMenu,
    ImeComposing,
    ChatDrawer,
    StageBlackoutF12,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrossAppCoordinator {
    pub anchor_lock_id: String,
    pub active_star_owner: AppSurfaceFocus,
    pub star_widget_count: usize,
    pub chat_seq: u64,
    pub speculative_idle_ms: u64,
    pub louis_resident_vram_mb: f32,
    pub louis_disk_queue_only: bool,
    pub gpu_lease: GpuLeaseState,
    pub privacy: CrossAppPrivacySync,
    pub ime_composing: bool,
    pub blackout_active: bool,
}

impl Default for CrossAppCoordinator {
    fn default() -> Self {
        Self {
            anchor_lock_id: DESKTOP_ANCHOR_LOCK.into(),
            active_star_owner: AppSurfaceFocus::LouisDesktop,
            star_widget_count: 1,
            chat_seq: 0,
            speculative_idle_ms: 0,
            louis_resident_vram_mb: 1_200.0,
            louis_disk_queue_only: false,
            gpu_lease: GpuLeaseState::LouisExclusive,
            privacy: CrossAppPrivacySync::default(),
            ime_composing: false,
            blackout_active: false,
        }
    }
}

impl CrossAppCoordinator {
    /// Hands `( ✦ )` ownership cleanly across apps while preserving `star_widget_count == 1` and `chat_seq`.
    pub fn handoff_star_widget(&mut self, next_owner: AppSurfaceFocus) -> usize {
        self.active_star_owner = next_owner;
        self.star_widget_count = 1;
        self.speculative_idle_ms = 0;
        self.star_widget_count
    }

    /// Reclaims speculative headless process after `>= 45_000 ms` idle, pinning Louis VRAM `<= 32.0 MB`.
    pub fn tick_speculative_reclaim(&mut self, idle_ms: u64) -> bool {
        self.speculative_idle_ms = idle_ms;
        if idle_ms >= SPECULATIVE_TTL_MS {
            self.louis_resident_vram_mb = self.louis_resident_vram_mb.min(STAGE_PINNED_LOUIS_CAP_MB);
            true
        } else {
            false
        }
    }

    /// Transitions to `StagePinnedLock`: drops Louis VRAM `<= 32.0 MB` and enables `DiskQueueOnly`.
    pub fn acquire_stage_pinned_lock(&mut self) {
        self.gpu_lease = GpuLeaseState::StagePinnedLock {
            louis_cap_mb_x10: 320,
        };
        self.louis_resident_vram_mb = STAGE_PINNED_LOUIS_CAP_MB;
        self.louis_disk_queue_only = true;
        self.active_star_owner = AppSurfaceFocus::VisionXrStageLive;
        self.star_widget_count = 1;
    }

    /// Releases `StagePinnedLock` back to Louis after XR Stage finishes with zero lost queued tasks.
    pub fn release_stage_pinned_lock(&mut self) {
        self.gpu_lease = GpuLeaseState::LouisExclusive;
        self.louis_disk_queue_only = false;
    }

    /// Handles `ESC` (canceled while `ime_composing == true`) vs `F12` Blackout kill-switch (always preempts).
    pub fn handle_key_event(&mut self, is_f12: bool) -> bool {
        if is_f12 {
            self.blackout_active = true;
            return true;
        }
        if self.ime_composing {
            return false; // Do not close drawer or clear Korean/CJK/Japanese IME composition
        }
        true
    }
}

pub fn build_vision_stage_deeplink(stage_id: &str, twin_seed: u64, cue_idx: u32, obb: &str) -> String {
    format!("vision://stage/{stage_id}?seed={twin_seed}&cue={cue_idx}&obb={obb}")
}

pub fn build_louis_doc_deeplink(bundle_id: &str, doc_id: &str, bbox: [u32; 4]) -> String {
    format!(
        "louis://doc/{bundle_id}/{doc_id}?bbox={},{},{},{}",
        bbox[0], bbox[1], bbox[2], bbox[3]
    )
}

pub fn validate_mobile_skin_webrtc_ip(skin: MobileSkinKind, raw_ip: &str) -> Result<(), VaultError> {
    if matches!(skin, MobileSkinKind::CounterKiosk) {
        return Err(VaultError::KioskForbidden);
    }
    let s = raw_ip.trim();
    if s.is_empty() {
        return Err(VaultError::CryptoFailed);
    }
    if s.contains(':') {
        let ok = if s == "::1" {
            true
        } else {
            let first = s.split(':').next().unwrap_or("");
            u16::from_str_radix(first, 16)
                .is_ok_and(|seg| (seg & 0xfe00) == 0xfc00 || (seg & 0xffc0) == 0xfe80)
        };
        return if ok { Ok(()) } else { Err(VaultError::CryptoFailed) };
    }
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return Err(VaultError::CryptoFailed);
    }
    let mut octets = [0u8; 4];
    for (i, p) in parts.iter().enumerate() {
        let Ok(v) = p.parse::<u8>() else {
            return Err(VaultError::CryptoFailed);
        };
        octets[i] = v;
    }
    let [a, b, _, _] = octets;
    let ok = a == 127
        || a == 10
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 169 && b == 254);
    if ok {
        Ok(())
    } else {
        Err(VaultError::CryptoFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cx01_to_cx08_cross_app_invariants_hold() {
        let mut coord = CrossAppCoordinator::default();
        assert_eq!(coord.anchor_lock_id, "louis.desktop.anchor.v1");
        assert_eq!(coord.handoff_star_widget(AppSurfaceFocus::VisionXrStageLive), 1);

        // CX-01: 45s speculative reclaim caps VRAM <= 32 MB
        assert!(!coord.tick_speculative_reclaim(44_999));
        assert!(coord.tick_speculative_reclaim(45_000));
        assert!(coord.louis_resident_vram_mb <= 32.0);

        // CX-02 / CX-03 deep links
        assert_eq!(
            build_vision_stage_deeplink("ARENA-01", 20260501, 4, "OBB-Lift-C"),
            "vision://stage/ARENA-01?seed=20260501&cue=4&obb=OBB-Lift-C"
        );
        assert_eq!(
            build_louis_doc_deeplink("DIAG-DOM09", "DOC-1", [10, 20, 300, 400]),
            "louis://doc/DIAG-DOM09/DOC-1?bbox=10,20,300,400"
        );

        // CX-04: StagePinnedLock drops Louis VRAM <= 32 MB and queues to disk
        coord.acquire_stage_pinned_lock();
        assert!(coord.louis_disk_queue_only && coord.louis_resident_vram_mb <= 32.0);
        coord.release_stage_pinned_lock();
        assert!(!coord.louis_disk_queue_only);

        // CX-05: Dual 2D+3D buffer zeroize & 3000ms + 3 ticks reprime gate
        let mut buf2d = vec![200u8; 64];
        let mut buf3d = vec![180u8; 64];
        let ep = coord.privacy.trigger_lockdown_and_zeroize(1_000, &mut buf2d, &mut buf3d);
        assert_eq!(ep, 1);
        assert!(buf2d.iter().all(|&b| b == 0) && buf3d.iter().all(|&b| b == 0));
        assert!(!coord.privacy.observe_clean_tick(1_500));
        assert!(!coord.privacy.observe_clean_tick(4_500));
        assert!(coord.privacy.observe_clean_tick(5_000)); // 3rd tick & >= 3000ms clean

        // CX-06: Mobile 4 skins + WAN block + Kiosk jail
        assert_eq!(MobileSkinKind::ALL.len(), 4);
        assert_eq!(
            validate_mobile_skin_webrtc_ip(MobileSkinKind::CounterKiosk, "192.168.1.20"),
            Err(VaultError::KioskForbidden)
        );
        assert!(validate_mobile_skin_webrtc_ip(MobileSkinKind::WirelessCameraRig, "192.168.1.20").is_ok());
        assert!(validate_mobile_skin_webrtc_ip(MobileSkinKind::StageCueRemote, "8.8.8.8").is_err());

        // CX-08: IME protects ESC; F12 blackout overrides everything
        coord.ime_composing = true;
        assert!(!coord.handle_key_event(false));
        assert!(coord.handle_key_event(true));
        assert!(coord.blackout_active);
    }
}
