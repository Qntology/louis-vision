//! Dual-app peer bridge (`127.0.0.1:8787` loopback-only) for `apps/vision`:
//! - Owns `#lv-star-root` locally when launched directly before `louis-desktop` (0 duplicate anchors).
//! - Sends `RequestGpuLease(CoexistSim | StagePinnedLock)` before entering Sim or XR Stage Live.
//! - Subscribes to global `BroadcastPrivacyEpoch` to blackout 3D display surfaces.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub const ROUTER_LOOPBACK_PORT: u16 = 8787;
pub const DESKTOP_ANCHOR_LOCK: &str = "louis.desktop.anchor.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerSurfaceMode {
    SimSandbox,
    XrStageLive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerGpuLease {
    CoexistSim,
    StagePinnedLock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerRouterState {
    pub host_connected: bool,
    pub owns_star_fallback: bool,
    pub active_mode: PeerSurfaceMode,
    pub lease: PeerGpuLease,
    pub privacy_epoch: u64,
    pub privacy_locked: bool,
    pub louis_resident_mb: f32,
}

impl Default for PeerRouterState {
    fn default() -> Self {
        Self {
            host_connected: false,
            owns_star_fallback: true,
            active_mode: PeerSurfaceMode::SimSandbox,
            lease: PeerGpuLease::CoexistSim,
            privacy_epoch: 0,
            privacy_locked: false,
            louis_resident_mb: 0.0,
        }
    }
}

pub fn validate_loopback_peer(addr: SocketAddr) -> Result<()> {
    if addr.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) && !addr.ip().is_loopback() {
        bail!("router_peer forbids non-loopback address {addr}");
    }
    Ok(())
}

impl PeerRouterState {
    pub fn on_host_connected(&mut self, host_owns_overlay: bool) {
        self.host_connected = true;
        // When vision is fullscreen, vision renders the single in-DOM ( ✦ ) widget;
        // when host overlay is active outside vision fullscreen, fallback stays off.
        self.owns_star_fallback = !host_owns_overlay;
    }

    pub fn request_stage_live(&mut self, host_ack_louis_mb: f32) -> PeerGpuLease {
        self.active_mode = PeerSurfaceMode::XrStageLive;
        self.lease = PeerGpuLease::StagePinnedLock;
        self.owns_star_fallback = true;
        self.louis_resident_mb = host_ack_louis_mb;
        self.lease
    }

    pub fn apply_privacy_epoch(&mut self, epoch: u64, locked: bool) {
        self.privacy_epoch = epoch;
        self.privacy_locked = locked;
    }
}

#[derive(Debug, Default)]
pub struct ManagedPeerRouter(pub std::sync::Mutex<PeerRouterState>);

#[tauri::command]
pub fn peer_state(state: tauri::State<'_, ManagedPeerRouter>) -> PeerRouterState {
    state.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

#[tauri::command]
pub fn peer_request_stage_live(
    state: tauri::State<'_, ManagedPeerRouter>,
    host_ack_louis_mb: f32,
) -> PeerRouterState {
    let mut g = state.0.lock().unwrap_or_else(|p| p.into_inner());
    let _ = g.request_stage_live(host_ack_louis_mb);
    g.clone()
}

#[tauri::command]
pub fn peer_apply_privacy_epoch(
    state: tauri::State<'_, ManagedPeerRouter>,
    epoch: u64,
    locked: bool,
) -> PeerRouterState {
    let mut g = state.0.lock().unwrap_or_else(|p| p.into_inner());
    g.apply_privacy_epoch(epoch, locked);
    g.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_rejects_wan_and_tracks_stage_lock() {
        assert!(validate_loopback_peer("127.0.0.1:8787".parse().unwrap()).is_ok());
        assert!(validate_loopback_peer("8.8.8.8:8787".parse().unwrap()).is_err());

        let mut peer = PeerRouterState::default();
        assert_eq!(peer.request_stage_live(0.0), PeerGpuLease::StagePinnedLock);
        assert!(peer.louis_resident_mb <= 32.0);
        peer.apply_privacy_epoch(3, true);
        assert!(peer.privacy_locked);
    }
}
