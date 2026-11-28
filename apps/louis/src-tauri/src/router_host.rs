//! Single-Anchor (`louis.desktop.anchor.v1`) loopback router (`127.0.0.1:8787`),
//! 4-direction intent & file routing (`Speculative 45s` vs `ActiveUse`),
//! 3-tier GPU Lease Handoff (`LouisExclusive`, `CoexistSim`, `StagePinnedLock`),
//! global `privacy_epoch` broadcast, and Win32 `LayerFlags` transition table.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use premium_discount_window::{ex_style, LayerFlags, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT};
use serde::{Deserialize, Serialize};

pub const DESKTOP_ANCHOR_LOCK: &str = "louis.desktop.anchor.v1";
pub const ROUTER_LOOPBACK_PORT: u16 = 8787;
pub const VISION_MCP_PORT: u16 = 8788;
pub const SPECULATIVE_TTL_SECS: u64 = 45;
pub const STAGE_LEASE_RELEASE_TIMEOUT_MS: u64 = 1500;
pub const STAGE_VRAM_CAP_MB: f32 = 32.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum N2sfWireGrade {
    Open,
    Sensitive,
    Classified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRoleWire {
    PersonalPhone,
    CounterKiosk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveSurfaceOwner {
    LouisDesktopOverlay,
    LouisDocsStudio,
    LouisField3dLens,
    VisionSimSandbox,
    VisionXrStageLive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuLeaseTier {
    LouisExclusive,
    CoexistSim,
    StagePinnedLock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarmLeaseKind {
    LocalOnly,
    Speculative { ttl_secs: u64 },
    ActiveUse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum LoopbackWireMsg {
    Handshake {
        pid: u32,
        role: lv_router::vault::DeviceRole,
        n2sf: N2sfWireGrade,
        owner: ActiveSurfaceOwner,
    },
    RequestGpuLease {
        target: GpuLeaseTier,
        requester_pid: u32,
    },
    AckGpuLease {
        granted: GpuLeaseTier,
        louis_resident_mb: f32,
    },
    HandoffStarWidget {
        to: ActiveSurfaceOwner,
        last_chat_seq: u64,
        active_tasks: usize,
    },
    OpenDeepLink {
        uri: String,
    },
    BroadcastPrivacyEpoch {
        epoch: u64,
        locked: bool,
        reason: String,
    },
    Heartbeat {
        surface: ActiveSurfaceOwner,
        lease: GpuLeaseTier,
    },
}

/// Rejects any non-loopback address for inter-app control plane (`127.0.0.1:8787` / `:8788`).
pub fn ensure_loopback_bind(addr: SocketAddr) -> Result<()> {
    if addr.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) && !addr.ip().is_loopback() {
        bail!("dual-app router refuses non-loopback bind address: {addr}");
    }
    Ok(())
}

/// Computes deterministic Win32 `GWL_EXSTYLE` mask for each desktop overlay phase:
/// - ghost idle: `0x0808_00A8`
/// - hover/drag: `0x0808_0088`
/// - typing/IME: `0x0008_0088`
pub fn overlay_ex_style_for(click_through: bool, topmost: bool, typing: bool) -> u32 {
    let flags = LayerFlags {
        click_through,
        topmost,
        typing,
    };
    let mask = ex_style(flags);
    let _expected_ghost =
        WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
    mask
}

/// Locates companion `louis-vision-app` binary across CLI arg, env, exe-sibling sidecar (`externalBin`), and install paths.
pub fn find_vision_peer(cli_arg: Option<&Path>, current_exe: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = cli_arg.filter(|p| p.is_file()) {
        return Some(p.to_path_buf());
    }
    if let Ok(env_p) = std::env::var("LV_VISION_APP") {
        let p = PathBuf::from(env_p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(exe) = current_exe.and_then(Path::parent) {
        for cand in ["louis-vision-app.exe", "louis-vision-app"] {
            let p = exe.join(cand);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    pub owner: ActiveSurfaceOwner,
    pub warm_lease: WarmLeaseKind,
    pub gpu_lease: GpuLeaseTier,
    pub show_vision_chip: bool,
    pub blocked_kiosk: bool,
}

/// Deterministic 0 ms router selecting surface owner, process lifecycle, and GPU lease tier.
pub fn route_intent(
    input: &str,
    files: &[&str],
    role: lv_router::vault::DeviceRole,
) -> RouteDecision {
    let lower = input.to_ascii_lowercase();
    let has_ext = |exts: &[&str]| {
        files.iter().any(|f| {
            let fl = f.to_ascii_lowercase();
            exts.iter().any(|e| fl.ends_with(e))
        })
    };

    let is_doc_ext = has_ext(&[".hwpx", ".hwp", ".docx", ".xlsx", ".xls", ".csv", ".pdf", ".xml", ".mhtml"]);
    let is_doc_noun = ["기성내역서", "청구서", "계약서", "명세서", "송장", "세금계산서", "보고서", "증빙", "내역서"]
        .iter()
        .any(|k| lower.contains(k));

    if lower.starts_with("/stage") || lower.contains("본공연") || lower.contains("stage live") {
        return RouteDecision {
            owner: ActiveSurfaceOwner::VisionXrStageLive,
            warm_lease: WarmLeaseKind::ActiveUse,
            gpu_lease: GpuLeaseTier::StagePinnedLock,
            show_vision_chip: false,
            blocked_kiosk: false,
        };
    }

    // D-11: Explicit document signals (extension, /docs, or doc-form nouns when not explicitly /sim)
    // take precedence over generic Sim keywords ("로봇 용접 설비 기성내역서 찾아줘" -> LouisDocsStudio).
    if lower.starts_with("/docs") || is_doc_ext || (is_doc_noun && !lower.starts_with("/sim")) {
        return RouteDecision {
            owner: ActiveSurfaceOwner::LouisDocsStudio,
            warm_lease: WarmLeaseKind::LocalOnly,
            gpu_lease: GpuLeaseTier::LouisExclusive,
            show_vision_chip: false,
            blocked_kiosk: false,
        };
    }

    if lower.starts_with("/sim")
        || has_ext(&[".glb"])
        || ["로봇", "지게차", "그리퍼", "큐시트", "영점"]
            .iter()
            .any(|k| lower.contains(k))
    {
        return RouteDecision {
            owner: ActiveSurfaceOwner::VisionSimSandbox,
            warm_lease: WarmLeaseKind::ActiveUse,
            gpu_lease: GpuLeaseTier::CoexistSim,
            show_vision_chip: false,
            blocked_kiosk: false,
        };
    }

    if has_ext(&[".urdf"]) || lower.contains("리허설") || lower.contains("최대 오차") {
        return RouteDecision {
            owner: ActiveSurfaceOwner::LouisDesktopOverlay,
            warm_lease: WarmLeaseKind::Speculative {
                ttl_secs: SPECULATIVE_TTL_SECS,
            },
            gpu_lease: GpuLeaseTier::CoexistSim,
            show_vision_chip: false,
            blocked_kiosk: false,
        };
    }

    if lower.starts_with("/lens") || lower.contains("명판 촬영") || lower.contains("현장 검측") {
        if role == lv_router::vault::DeviceRole::CounterKiosk {
            return RouteDecision {
                owner: ActiveSurfaceOwner::LouisDesktopOverlay,
                warm_lease: WarmLeaseKind::LocalOnly,
                gpu_lease: GpuLeaseTier::LouisExclusive,
                show_vision_chip: false,
                blocked_kiosk: true,
            };
        }
        return RouteDecision {
            owner: ActiveSurfaceOwner::LouisField3dLens,
            warm_lease: WarmLeaseKind::ActiveUse,
            gpu_lease: GpuLeaseTier::LouisExclusive,
            show_vision_chip: false,
            blocked_kiosk: false,
        };
    }

    let is_border_media = has_ext(&[".png", ".jpg", ".jpeg", ".mp4", ".mkv"]);
    let is_doc_or_cmd = lower.starts_with("/docs")
        || has_ext(&[".hwpx", ".hwp", ".docx", ".xlsx", ".xls", ".csv", ".pdf", ".xml", ".mhtml"]);

    RouteDecision {
        owner: if is_doc_or_cmd {
            ActiveSurfaceOwner::LouisDocsStudio
        } else {
            ActiveSurfaceOwner::LouisDesktopOverlay
        },
        warm_lease: WarmLeaseKind::LocalOnly,
        gpu_lease: GpuLeaseTier::LouisExclusive,
        show_vision_chip: is_border_media,
        blocked_kiosk: false,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostRouterState {
    pub active_surface: ActiveSurfaceOwner,
    pub gpu_lease: GpuLeaseTier,
    pub louis_resident_mb: f32,
    pub louis_overlay_visible: bool,
    pub vision_star_visible: bool,
    pub last_chat_seq: u64,
    pub privacy_epoch: u64,
    pub privacy_locked: bool,
    pub ingest_disk_queue_only: bool,
}

impl Default for HostRouterState {
    fn default() -> Self {
        Self {
            active_surface: ActiveSurfaceOwner::LouisDesktopOverlay,
            gpu_lease: GpuLeaseTier::LouisExclusive,
            louis_resident_mb: 1850.0,
            louis_overlay_visible: true,
            vision_star_visible: false,
            last_chat_seq: 0,
            privacy_epoch: 0,
            privacy_locked: false,
            ingest_disk_queue_only: false,
        }
    }
}

impl HostRouterState {
    /// Enforces strictly `active_star_widgets == 1` across `louis` and `vision` and honors GPU lease transitions.
    pub fn apply_wire(&mut self, msg: LoopbackWireMsg) -> LoopbackWireMsg {
        match msg {
            LoopbackWireMsg::Handshake { owner, .. } => {
                self.handoff_to(owner);
                LoopbackWireMsg::Heartbeat {
                    surface: self.active_surface,
                    lease: self.gpu_lease,
                }
            }
            LoopbackWireMsg::RequestGpuLease { target, .. } => {
                self.gpu_lease = target;
                match target {
                    GpuLeaseTier::LouisExclusive => {
                        self.louis_resident_mb = 1850.0;
                        self.ingest_disk_queue_only = false;
                    }
                    GpuLeaseTier::CoexistSim => {
                        self.louis_resident_mb = 1150.0;
                        self.ingest_disk_queue_only = false;
                    }
                    GpuLeaseTier::StagePinnedLock => {
                        self.louis_resident_mb = 0.0;
                        self.ingest_disk_queue_only = true;
                        self.handoff_to(ActiveSurfaceOwner::VisionXrStageLive);
                    }
                }
                LoopbackWireMsg::AckGpuLease {
                    granted: self.gpu_lease,
                    louis_resident_mb: self.louis_resident_mb,
                }
            }
            LoopbackWireMsg::HandoffStarWidget { to, last_chat_seq, .. } => {
                self.last_chat_seq = last_chat_seq.max(self.last_chat_seq);
                self.handoff_to(to);
                LoopbackWireMsg::Heartbeat {
                    surface: self.active_surface,
                    lease: self.gpu_lease,
                }
            }
            LoopbackWireMsg::BroadcastPrivacyEpoch { epoch, locked, reason } => {
                self.privacy_epoch = epoch.max(self.privacy_epoch + 1);
                self.privacy_locked = locked;
                LoopbackWireMsg::BroadcastPrivacyEpoch {
                    epoch: self.privacy_epoch,
                    locked: self.privacy_locked,
                    reason,
                }
            }
            other => other,
        }
    }

    pub fn active_star_count(&self) -> usize {
        usize::from(self.louis_overlay_visible) + usize::from(self.vision_star_visible)
    }

    fn handoff_to(&mut self, to: ActiveSurfaceOwner) {
        self.active_surface = to;
        match to {
            ActiveSurfaceOwner::VisionSimSandbox | ActiveSurfaceOwner::VisionXrStageLive => {
                self.louis_overlay_visible = false;
                self.vision_star_visible = true;
            }
            ActiveSurfaceOwner::LouisDesktopOverlay
            | ActiveSurfaceOwner::LouisDocsStudio
            | ActiveSurfaceOwner::LouisField3dLens => {
                self.louis_overlay_visible = true;
                self.vision_star_visible = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lv_router::vault::DeviceRole;

    #[test]
    fn layer_flags_loopback_routing_and_single_anchor_invariants() {
        // Ex-style masks matching Win32 Part 7 table
        assert_eq!(overlay_ex_style_for(true, true, false), 0x0808_00A8);
        assert_eq!(overlay_ex_style_for(false, true, false), 0x0808_0088);
        assert_eq!(overlay_ex_style_for(false, true, true), 0x0008_0088);

        assert!(ensure_loopback_bind("127.0.0.1:8787".parse().unwrap()).is_ok());
        assert!(ensure_loopback_bind("8.8.8.8:8787".parse().unwrap()).is_err());

        let r_spec = route_intent("방금 리허설 최대 오차 알려줘", &[], DeviceRole::PersonalPhone);
        assert_eq!(r_spec.warm_lease, WarmLeaseKind::Speculative { ttl_secs: 45 });

        let r_media = route_intent("송장 스캔", &["inv.png"], DeviceRole::PersonalPhone);
        assert!(r_media.show_vision_chip);

        let r_kiosk_lens = route_intent("/lens", &[], DeviceRole::CounterKiosk);
        assert!(r_kiosk_lens.blocked_kiosk);

        let mut st = HostRouterState::default();
        assert_eq!(st.active_star_count(), 1);
        let ack = st.apply_wire(LoopbackWireMsg::RequestGpuLease {
            target: GpuLeaseTier::StagePinnedLock,
            requester_pid: 42,
        });
        assert_eq!(st.active_star_count(), 1);
        assert!(st.ingest_disk_queue_only);
        assert!(matches!(ack, LoopbackWireMsg::AckGpuLease { louis_resident_mb, .. } if louis_resident_mb <= STAGE_VRAM_CAP_MB));
    }
}
