//! Layer 2.5 zero-network 3D-to-2D metric homography bridge (strictly links `lv-doc` with `default-features = false`).

pub mod anchor3d;
pub mod doc_bridge;
pub mod gate;
pub mod homography;
#[cfg(feature = "screen")]
pub mod screen_bridge;
