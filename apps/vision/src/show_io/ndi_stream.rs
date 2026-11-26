//! Local RFC1918 NDI Video + Alpha (`BGRA8`) broadcast stream builder.

use std::net::SocketAddr;

use anyhow::{ensure, Result};
use lv_spatial::frame::{AlphaMatte, Rgb};

use crate::validate_local_socket;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NdiVideoAlphaFrame {
    pub stream_name: String,
    pub endpoint: SocketAddr,
    pub width: u32,
    pub height: u32,
    pub fourcc: &'static str,
    pub bgra: Vec<u8>,
}

/// Packs straight RGB + Joint Bilateral `AlphaMatte` into an `NDI_BGRA` local-LAN broadcast frame.
pub fn pack_ndi_video_alpha(
    stream_name: impl Into<String>,
    endpoint: SocketAddr,
    rgb: &Rgb,
    matte: &AlphaMatte,
) -> Result<NdiVideoAlphaFrame> {
    let endpoint = validate_local_socket(endpoint, "NDI broadcast LAN endpoint")?;
    ensure!(
        rgb.width == matte.width && rgb.height == matte.height,
        "NDI RGB ({}x{}) and AlphaMatte ({}x{}) must match",
        rgb.width,
        rgb.height,
        matte.width,
        matte.height
    );
    let mut bgra = Vec::with_capacity((rgb.width * rgb.height * 4) as usize);
    for (px, &a) in rgb.data.chunks_exact(3).zip(&matte.alpha) {
        bgra.push(px[2]);
        bgra.push(px[1]);
        bgra.push(px[0]);
        bgra.push(a);
    }
    Ok(NdiVideoAlphaFrame {
        stream_name: stream_name.into(),
        endpoint,
        width: rgb.width,
        height: rgb.height,
        fourcc: "BGRA",
        bgra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_local_ndi_bgra_alpha_and_rejects_wan() {
        let rgb = Rgb::new(2, 1, vec![10, 20, 30, 40, 50, 60]);
        let matte = AlphaMatte { width: 2, height: 1, alpha: vec![200, 55] };
        assert!(pack_ndi_video_alpha("Stage1", "8.8.4.4:5961".parse().unwrap(), &rgb, &matte).is_err());

        let frame = pack_ndi_video_alpha("Stage1", "192.168.1.50:5961".parse().unwrap(), &rgb, &matte).unwrap();
        assert_eq!(frame.fourcc, "BGRA");
        assert_eq!(&frame.bgra[..4], &[30, 20, 10, 200]);
        assert_eq!(&frame.bgra[4..8], &[60, 50, 40, 55]);
    }
}
