//! Procedural 6-axis synthetic document, pixel, and 3D fixture builders (100% in-memory, zero customer files).

pub mod audio_patch_synth;
pub mod burst_concurrent;
pub mod digital_clean;
pub mod digital_hostile;
#[cfg(feature = "field-3d")]
pub mod field_3d_planar;
pub mod privacy_contaminated;
pub mod scan_2d_degraded;
