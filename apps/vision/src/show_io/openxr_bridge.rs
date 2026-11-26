//! OpenXR `STAGE` reference space anchor mapping performer 3D tracks to HMD overlays.

use lv_spatial::zkey::PerformerTrack3D;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XrSandboxAnchorPose {
    pub track_id: u32,
    pub position_m: [f32; 3],
    pub velocity_mps: [f32; 3],
    pub orientation_xyzw: [f32; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XrSandboxAnchorFrame {
    pub reference_space: &'static str,
    pub frame_index: u64,
    pub anchors: Vec<XrSandboxAnchorPose>,
}

/// Projects metric sandbox tracks directly into OpenXR `STAGE` coordinates (no local origin drift).
pub fn map_to_openxr_sandbox_space(frame_index: u64, performers: &[PerformerTrack3D]) -> XrSandboxAnchorFrame {
    let anchors = performers
        .iter()
        .map(|p| XrSandboxAnchorPose {
            track_id: p.id,
            position_m: p.pos_m,
            velocity_mps: p.vel_mps,
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        })
        .collect();
    XrSandboxAnchorFrame {
        reference_space: "STAGE",
        frame_index,
        anchors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_sandbox_performers_to_openxr_sandbox_space() {
        let p = PerformerTrack3D {
            id: 4,
            pos_m: [1.0, 1.2, -3.4],
            vel_mps: [0.2, 0.0, 0.1],
            bbox_px: [10, 20, 30, 40],
            pixel_count: 100,
            missed_frames: 0,
        };
        let xr = map_to_openxr_sandbox_space(9, &[p]);
        assert_eq!(xr.reference_space, "STAGE");
        assert_eq!(xr.anchors.len(), 1);
        assert_eq!(xr.anchors[0].position_m, [1.0, 1.2, -3.4]);
    }
}
