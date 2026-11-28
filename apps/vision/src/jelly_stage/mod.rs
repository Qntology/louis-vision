//! BPM-driven Jelly / Keith Haring Stage World (`vision-sandbox v1.8`).
//!
//! Strictly enforces Rule 6 (`HaringDancerRig` flat interior purity `>= 99.5%`,
//! `0 px` `#000000` outline leakage, `>= 4` air-gapped floating ticks) and volume-preserving
//! (`det(S) = 1.0`) squash-and-stretch capsules.

pub mod motion;
pub mod rig_bridge;

use anyhow::Result;
use sandbox_core::rig::haring_dancer::HaringDancerRig;
use serde::{Deserialize, Serialize};

pub use motion::{evaluate_bpm_motif, BpmMotif, HeadPropKind, JellyPoseFrame, JellySdfCapsule};
pub use rig_bridge::{audit_haring_rig_invariants, build_rig_from_jelly_actor, HaringRigAudit, JellyActorSpec};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JellyStageSnapshot {
    pub t_s: f32,
    pub bpm: f32,
    pub poses: Vec<(String, JellyPoseFrame)>,
    pub rigs: Vec<HaringDancerRig>,
}

pub struct JellyStageWorld {
    pub bpm: f32,
    pub actors: Vec<JellyActorSpec>,
}

impl JellyStageWorld {
    pub fn new(bpm: f32) -> Self {
        Self {
            bpm: bpm.clamp(40.0, 220.0),
            actors: Vec::new(),
        }
    }

    pub fn spawn_actor(&mut self, actor: JellyActorSpec) {
        self.actors.push(actor);
    }

    pub fn evaluate_at(&self, t_s: f32) -> JellyStageSnapshot {
        let mut poses = Vec::with_capacity(self.actors.len());
        let mut rigs = Vec::with_capacity(self.actors.len());
        for a in &self.actors {
            let pose = evaluate_bpm_motif(a.motif, a.origin_xz, a.bpm, t_s);
            let rig = build_rig_from_jelly_actor(a, pose.beat_env);
            poses.push((a.actor_id.clone(), pose));
            rigs.push(rig);
        }
        JellyStageSnapshot {
            t_s,
            bpm: self.bpm,
            poses,
            rigs,
        }
    }

    pub fn audit_all_actors(&self, t_s: f32) -> Result<Vec<HaringRigAudit>> {
        let snap = self.evaluate_at(t_s);
        snap.rigs.iter().map(audit_haring_rig_invariants).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jelly_stage_preserves_unit_volume_and_rule6_haring_invariants() {
        let mut world = JellyStageWorld::new(128.0);
        world.spawn_actor(JellyActorSpec {
            actor_id: "HARING-01".into(),
            wallet_0x: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e".into(),
            origin_xz: [-1.5, 0.0],
            bpm: 128.0,
            motif: BpmMotif::GrooveWave,
            head_prop: HeadPropKind::Crown,
        });
        world.spawn_actor(JellyActorSpec {
            actor_id: "HARING-02".into(),
            wallet_0x: "0x1111111111111111111111111111111111111111".into(),
            origin_xz: [1.5, 0.0],
            bpm: 128.0,
            motif: BpmMotif::ElasticHop,
            head_prop: HeadPropKind::Headphones,
        });
        world.spawn_actor(JellyActorSpec {
            actor_id: "HARING-03".into(),
            wallet_0x: "0x2222222222222222222222222222222222222222".into(),
            origin_xz: [0.0, 2.0],
            bpm: 128.0,
            motif: BpmMotif::MoveToXz { target_xz: [3.0, -1.0] },
            head_prop: HeadPropKind::Halo,
        });

        let snap = world.evaluate_at(2.5);
        assert_eq!(snap.poses.len(), 3);
        for (_, pose) in &snap.poses {
            for cap in &pose.capsules {
                assert!(
                    (cap.scale_det() - 1.0).abs() < 1e-5,
                    "capsule {} violated det(S)=1: {}",
                    cap.id,
                    cap.scale_det()
                );
            }
        }
        let audits = world.audit_all_actors(2.5).unwrap();
        assert_eq!(audits.len(), 3);
        for a in audits {
            assert!(a.passed);
            assert!(a.interior_purity_pct >= 99.5);
            assert_eq!(a.outline_leak_px, 0);
            assert!(a.floating_tick_count >= 4);
        }
    }
}
