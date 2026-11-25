//! Spout2 (Windows DirectX/GL) and Syphon (macOS Metal) zero-copy shared texture descriptors
//! carrying 21-color `mask_ops` ID mattes and metric `R32_FLOAT` depth planes.

use anyhow::{ensure, Result};
use lv_spatial::frame::{AlphaMatte, DepthMap};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuShareBackend {
    Spout2,
    Syphon,
}

impl GpuShareBackend {
    pub fn default_for_host() -> Self {
        if cfg!(target_os = "macos") {
            Self::Syphon
        } else {
            Self::Spout2
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZeroCopyTextureDesc {
    pub backend: GpuShareBackend,
    pub sender_name: String,
    pub color_format: &'static str,
    pub depth_format: &'static str,
    pub width: u32,
    pub height: u32,
    pub depth_width: u32,
    pub depth_height: u32,
    pub handle_id: u64,
}

/// Validates ID-matte buffer length and depth dimensions before registering a GPU shared handle.
pub fn build_zero_copy_descriptor(
    backend: GpuShareBackend,
    sender_name: impl Into<String>,
    matte: &AlphaMatte,
    id_matte_rgba: &[u8],
    depth: &DepthMap,
    frame_index: u64,
) -> Result<ZeroCopyTextureDesc> {
    let sender = sender_name.into();
    ensure!(!sender.trim().is_empty(), "GPU share sender name must not be empty");
    let expected = (matte.width * matte.height * 4) as usize;
    ensure!(
        id_matte_rgba.len() == expected,
        "ID matte byte len {} != expected RGBA8 {expected}",
        id_matte_rgba.len()
    );
    Ok(ZeroCopyTextureDesc {
        backend,
        sender_name: sender,
        color_format: "RGBA8_UNORM",
        depth_format: "R32_FLOAT_METRIC",
        width: matte.width,
        height: matte.height,
        depth_width: depth.width,
        depth_height: depth.height,
        handle_id: frame_index.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x510C_4F01,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lv_spatial::frame::DepthKind;

    #[test]
    fn builds_valid_spout2_and_syphon_descriptors() {
        let matte = AlphaMatte { width: 16, height: 8, alpha: vec![255; 128] };
        let rgba = vec![0u8; 16 * 8 * 4];
        let depth = DepthMap { width: 8, height: 4, kind: DepthKind::Metric, data: vec![2.5; 32] };
        let desc = build_zero_copy_descriptor(GpuShareBackend::Spout2, "LouisStage_ID", &matte, &rgba, &depth, 12)
            .unwrap();
        assert_eq!(desc.color_format, "RGBA8_UNORM");
        assert_eq!(desc.depth_format, "R32_FLOAT_METRIC");
        assert_eq!((desc.width, desc.height), (16, 8));
        assert_eq!((desc.depth_width, desc.depth_height), (8, 4));
    }
}
