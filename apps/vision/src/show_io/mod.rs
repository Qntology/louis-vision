//! Stage show output bridges: loopback/RFC1918 OSC 1.0 UDP, Spout2/Syphon GPU sharing,
//! local NDI Video+Alpha broadcast, and OpenXR `STAGE` reference anchors.

pub mod gpu_share;
pub mod ndi_stream;
pub mod openxr_bridge;
pub mod osc_udp;

pub use gpu_share::{build_zero_copy_descriptor, GpuShareBackend, ZeroCopyTextureDesc};
pub use ndi_stream::{pack_ndi_video_alpha, NdiVideoAlphaFrame};
pub use openxr_bridge::{map_to_openxr_stage_space, XrStageAnchorFrame, XrStageAnchorPose};
pub use osc_udp::{decode_osc_message, encode_osc_message, OscArg, OscUdpBridge, DEFAULT_OSC_ADDR};
