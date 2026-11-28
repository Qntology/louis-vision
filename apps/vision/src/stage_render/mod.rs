//! Stage rendering subsystem (`vision-sandbox v1.8`): multi-cam WebP ring buffer + adopted-cam tiled Cycles baker.

pub mod cycles_step;
pub mod ring_buffer;

pub use cycles_step::{CyclesBakeJob, CyclesBakeReport, FrameStepCycles};
pub use ring_buffer::{CamFrameEntry, MultiCamRingBuffer};

pub struct StageRenderCoordinator {
    pub ring: MultiCamRingBuffer,
    pub cycles: FrameStepCycles,
}

impl StageRenderCoordinator {
    pub fn new(ring_max_bytes: usize) -> Self {
        Self {
            ring: MultiCamRingBuffer::new(ring_max_bytes),
            cycles: FrameStepCycles::new(),
        }
    }

    pub fn stage_render_abort(&self) -> u64 {
        self.cycles.abort_now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_caps_memory_and_cycles_bakes_only_adopted_cam_with_fast_abort() {
        let mut coord = StageRenderCoordinator::new(8192);
        for f in 0..10u32 {
            coord.ring.push(CamFrameEntry {
                cam_id: "cam-foh-1".into(),
                frame_idx: f,
                width: 64,
                height: 64,
                webp_bytes: vec![0xAB; 2048],
            });
        }
        assert!(coord.ring.used_bytes() <= coord.ring.max_bytes());
        assert!(coord.ring.get("cam-foh-1", 9).is_some());
        assert!(coord.ring.get("cam-foh-1", 0).is_none());

        let job = CyclesBakeJob {
            slice_idx: 2,
            adopted_cam_id: "cam-foh-1".into(),
            candidate_cams: vec!["cam-foh-1".into(), "cam-side-l".into(), "cam-overhead".into()],
            width: 64,
            height: 64,
            tile_size: 32,
            samples: 16,
        };
        let (rep, rgba) = coord.cycles.bake_adopted_cam(&job);
        assert_eq!(rep.baked_cam_id, "cam-foh-1");
        assert_eq!(rep.skipped_cams, vec!["cam-side-l", "cam-overhead"]);
        assert_eq!(rep.tiles_completed, 4);
        assert!(!rep.aborted);
        assert_eq!(rgba.len(), 64 * 64 * 4);
        // Verify real raycast shading (non-uniform pixels and distinct camera perspectives)
        let unique_rgb: std::collections::BTreeSet<(u8, u8, u8)> = rgba
            .chunks_exact(4)
            .map(|p| (p[0], p[1], p[2]))
            .collect();
        assert!(unique_rgb.len() >= 12, "raycast output lacks shading diversity: {}", unique_rgb.len());

        let side_job = CyclesBakeJob {
            adopted_cam_id: "cam-side-l".into(),
            ..job.clone()
        };
        let (rep_side, rgba_side) = coord.cycles.bake_adopted_cam(&side_job);
        assert_eq!(rep_side.baked_cam_id, "cam-side-l");
        assert_ne!(rgba, rgba_side, "different adopted camera poses must render distinct frames");

        // Instant abort <= 16.6 ms (16_666 us)
        let abort_us = coord.stage_render_abort();
        assert!(abort_us <= 16_666, "abort took {abort_us} us > 16_666 us");
        let (rep_aborted, _) = coord.cycles.bake_adopted_cam(&job);
        assert!(rep_aborted.aborted);
        assert_eq!(rep_aborted.tiles_completed, 0);
    }
}
