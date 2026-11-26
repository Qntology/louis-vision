//! Layer 2 zero-network stereo, live XR Z-Keying, TIPSv2 monocular fallback, and 3D spatial tracking.

pub mod align;
pub mod calib;
pub mod camera;
pub mod decide;
pub mod depth;
pub mod egoexo;
pub mod fit3;
pub mod frame;
pub mod fusion;
pub mod item;
pub mod locate;
pub mod mask_ops;
pub mod motion_iou;
pub mod perforate;
pub mod planar;
pub mod pos2;
pub mod pyramid;
pub mod stairs;
pub mod stereo;
pub mod tips;
pub mod track;
pub mod triangulate;
pub mod zkey;

pub use depth::{DepthMetrics, DepthModel, DepthScene, SceneHeads};
pub use frame::{AlphaMatte, DepthKind, DepthMap, Mask, NormalMap, Rgb, SegMap};
pub use stereo::{sgbm, stereo_grid, stereo_sigma_z, SgbmCfg, SgbmDepth, StereoOut, StereoRig, StereoStats};
pub use track::{MaskTracker, PointPrompt, RegionCfg, RegionTracker, StereoSpatialPipeline, StereoSpatialTier, TrackedMask};
pub use zkey::{PerformerTrack3D, SandboxBackground, SandboxKeyOut, SandboxZKeyTracker, ZKeyCfg};
