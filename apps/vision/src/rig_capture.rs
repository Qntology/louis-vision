//! Stereo rig ingestion (`USB3` UVC uncompressed & high-bitrate LAN sync),
//! `STEREO_KEYFRAME_QUALITY >= 95.0` enforcement, and 5% Red Tally locked sandbox control.

use std::net::SocketAddr;

use anyhow::{bail, ensure, Result};
use lv_spatial::frame::Rgb;
use serde::{Deserialize, Serialize};

use crate::validate_local_socket;

/// Minimum compression quality accepted for stereo SBS disparity estimation.
pub const STEREO_KEYFRAME_QUALITY: f32 = 95.0;
/// Maximum tally luminance (%) allowed on wireless sandbox rigs so dark sandbox lighting stays unpolluted.
pub const MAX_SANDBOX_TALLY_LUMA_PCT: u8 = 5;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RigSource {
    /// Local uncompressed USB3 UVC stereo camera rig.
    UsbUvcUncompressed {
        device_id: String,
        width: u32,
        height: u32,
        fps: u32,
    },
    /// Air-gapped local LAN wireless stereo sync stream (requires quality >= 95.0 and loopback/RFC1918 IP).
    WirelessLanSync {
        quality: f32,
        endpoint: SocketAddr,
    },
}

impl RigSource {
    /// Validates transport locality and compression quality floor.
    pub fn validate(&self) -> Result<()> {
        match self {
            RigSource::UsbUvcUncompressed { width, height, fps, .. } => {
                ensure!(*width >= 2 && *width % 2 == 0, "SBS width must be even and >= 2, got {width}");
                ensure!(*height >= 2, "SBS height must be >= 2, got {height}");
                ensure!(*fps >= 30, "live sandbox capture requires >= 30 fps, got {fps}");
                Ok(())
            }
            RigSource::WirelessLanSync { quality, endpoint } => {
                validate_local_socket(*endpoint, "wireless sandbox rig LAN endpoint")?;
                ensure!(
                    quality.is_finite() && *quality >= STEREO_KEYFRAME_QUALITY,
                    "wireless stereo stream rejected: quality {quality:.2} < STEREO_KEYFRAME_QUALITY {STEREO_KEYFRAME_QUALITY:.1}"
                );
                Ok(())
            }
        }
    }
}

/// Splits a side-by-side stereo frame into `(left, right)` after checking `STEREO_KEYFRAME_QUALITY >= 95.0`.
pub fn decode_sbs_stereo_pair(sbs: &Rgb, quality: f32, source: &RigSource) -> Result<(Rgb, Rgb)> {
    source.validate()?;
    ensure!(
        quality.is_finite() && quality >= STEREO_KEYFRAME_QUALITY,
        "SBS stereo keyframe rejected: quality {quality:.2} < {STEREO_KEYFRAME_QUALITY:.1}"
    );
    ensure!(
        sbs.width >= 2 && sbs.width % 2 == 0,
        "SBS frame width must be even and >= 2, got {}",
        sbs.width
    );

    let eye_w = (sbs.width / 2) as usize;
    let h = sbs.height as usize;
    let full_stride = sbs.width as usize * 3;
    let eye_stride = eye_w * 3;

    let mut left_data = vec![0u8; eye_stride * h];
    let mut right_data = vec![0u8; eye_stride * h];
    for y in 0..h {
        let src_row = &sbs.data[y * full_stride..(y + 1) * full_stride];
        left_data[y * eye_stride..(y + 1) * eye_stride].copy_from_slice(&src_row[..eye_stride]);
        right_data[y * eye_stride..(y + 1) * eye_stride].copy_from_slice(&src_row[eye_stride..]);
    }
    Ok((
        Rgb::new(eye_w as u32, sbs.height, left_data),
        Rgb::new(eye_w as u32, sbs.height, right_data),
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TallyState {
    Standby,
    Preview,
    LiveOnAir,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WirelessCameraRig {
    pub id: String,
    pub touch_locked: bool,
    pub tally_red_luma_pct: u8,
    pub tally: TallyState,
    pub source: RigSource,
}

impl WirelessCameraRig {
    /// Creates a sandbox-ready wireless rig with touch locked and dim 5% red tally by default.
    pub fn new(id: impl Into<String>, source: RigSource) -> Result<Self> {
        source.validate()?;
        Ok(Self {
            id: id.into(),
            touch_locked: true,
            tally_red_luma_pct: MAX_SANDBOX_TALLY_LUMA_PCT,
            tally: TallyState::Standby,
            source,
        })
    }

    /// Sets tally state while enforcing the 5% dark-sandbox luminance cap.
    pub fn set_tally(&mut self, tally: TallyState, luma_pct: u8) -> Result<()> {
        ensure!(
            luma_pct <= MAX_SANDBOX_TALLY_LUMA_PCT,
            "sandbox tally luminance {luma_pct}% exceeds 5% dark-sandbox ceiling"
        );
        self.tally = tally;
        self.tally_red_luma_pct = luma_pct;
        Ok(())
    }

    /// Rejects accidental on-rig touchscreen taps unless FOH explicitly unlocked the rig.
    pub fn handle_touch_input(&self, control: &str) -> Result<()> {
        if self.touch_locked {
            bail!("rig '{}' touch input '{control}' rejected: sandbox touch lock active", self.id);
        }
        Ok(())
    }

    pub fn set_touch_lock(&mut self, locked: bool) {
        self.touch_locked = locked;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_sbs(eye_w: u32, h: u32) -> Rgb {
        let full_w = eye_w * 2;
        let mut data = vec![0u8; (full_w * h * 3) as usize];
        for y in 0..h {
            for x in 0..full_w {
                let i = ((y * full_w + x) * 3) as usize;
                let px = if x < eye_w { [10, 20, 30] } else { [200, 210, 220] };
                data[i..i + 3].copy_from_slice(&px);
            }
        }
        Rgb::new(full_w, h, data)
    }

    #[test]
    fn rejects_low_quality_and_wan_and_splits_sbs_cleanly() {
        let sbs = synthetic_sbs(32, 16);
        let usb = RigSource::UsbUvcUncompressed {
            device_id: "uvc-0".into(),
            width: 64,
            height: 16,
            fps: 60,
        };
        assert!(decode_sbs_stereo_pair(&sbs, 94.9, &usb).is_err());

        let wan = RigSource::WirelessLanSync {
            quality: 98.0,
            endpoint: "8.8.8.8:7001".parse().unwrap(),
        };
        assert!(decode_sbs_stereo_pair(&sbs, 98.0, &wan).is_err());

        let lan = RigSource::WirelessLanSync {
            quality: 96.0,
            endpoint: "192.168.10.21:7001".parse().unwrap(),
        };
        let (l, r) = decode_sbs_stereo_pair(&sbs, 95.0, &lan).unwrap();
        assert_eq!(l.width, 32);
        assert_eq!(r.width, 32);
        assert_eq!(l.px(0, 0), [10, 20, 30]);
        assert_eq!(r.px(0, 0), [200, 210, 220]);
    }

    #[test]
    fn wireless_rig_defaults_touch_locked_and_five_pct_tally() {
        let mut rig = WirelessCameraRig::new(
            "cam-sandbox-a",
            RigSource::WirelessLanSync {
                quality: 97.5,
                endpoint: "10.0.0.12:7001".parse().unwrap(),
            },
        )
        .unwrap();
        assert!(rig.touch_locked);
        assert_eq!(rig.tally_red_luma_pct, 5);
        assert!(rig.handle_touch_input("cue_trigger").is_err());
        assert!(rig.set_tally(TallyState::LiveOnAir, 12).is_err());
        rig.set_tally(TallyState::LiveOnAir, 5).unwrap();
        assert_eq!(rig.tally, TallyState::LiveOnAir);
    }
}
